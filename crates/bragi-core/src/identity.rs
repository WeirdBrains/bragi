//! This device's place in the user's Weirdnet: the user identity, its own node key and
//! certificate, and the linked-devices roster it shares with Morph and Ymir.
//!
//! Stored under `<dir>/identity/` (0700), secrets in 0600 files: `seed` (the user key seed, which a
//! device link delivers), `node.key`, `keys` (epoch keys) and the public `roster.json`. The OS
//! keystore comes later (B3), as in Morph.
//!
//! [`Identity::tick`] runs a `weirdnet-sync` pass against every linked device, so devices linked
//! or revoked elsewhere reach this one. Bragi has no synced records yet (history moves to the
//! context store in B4), so a pass carries the roster and the key chain only.
//!
//! Generic: this is the same identity store Ymir keeps (`ymir-core::membership`). The pull itself
//! (snapshot, pull every type in one session, merge back) is `weirdnet_sync::pull_types_shared`; the store around it still
//! belongs in `weirdnet-sync`, noted for the Weirdnet lead.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use weirdnet_net::user::NodeCert;
use weirdnet_net::{Keypair, Multiaddr, Node, PeerId, PublicKey, WEIRDNET};
use weirdnet_sync::{Device, EpochKeys, LogEntry, LogPage, RecordOptions, RecordValue, Roster, SyncState};

/// The sync type Bragi announces; the roster rides along with every pass.
pub const SYNC_TYPE: &str = "bragi.app";

/// The sync type of conversations, shared with Ymir (`ymir.context`): sealed under the roster's
/// key, so only the user's own devices can read it. See [`crate::context`].
pub const CONTEXT_TYPE: &str = "ymir.context";

/// What [`Identity::compact`] turns a dropped entry into: no record, no value, a type no app reads
/// or pulls. The entry keeps its node, seq and clock, so cursors and merges (which go by seq) still
/// line up and no sync pass brings the old value back.
pub const COMPACTED_TYPE: &str = "bragi.compacted";

/// What a link transfer carries besides the identity: the roster, in Morph's format, so a Morph
/// device and a Bragi device can link each other.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LinkSettings {
    #[serde(default)]
    pub roster: Roster,
    /// Where the user's other devices can be dialed (`/ip4/…/tcp/…/p2p/PEER`), from the device
    /// that sent the link: a device on another network (no mDNS, no DHT) finds its nodes this way.
    /// Ymir sends the same field; older senders leave it out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peers: Vec<String>,
}

impl LinkSettings {
    /// The `peers` a link transfer's settings carry (none when it has no such field or isn't JSON).
    pub fn peers_in(settings: &str) -> Vec<String> {
        serde_json::from_str::<LinkSettings>(settings).map(|s| s.peers).unwrap_or_default()
    }
}

pub type SyncNode = Node<Vec<u8>, Vec<u8>>;

pub struct Identity {
    dir: PathBuf,
    node_key: Keypair,
    state: Arc<Mutex<SyncState>>,
    /// When each device's wake cards were last pulled (B4). A pull costs a sync session of its
    /// own, and cards rarely change, so they come in every [`WAKE_PULL_EVERY`], not every pass.
    wake_pulled: Mutex<std::collections::HashMap<String, std::time::Instant>>,
}

/// How often a sync pass pulls a device's wake cards.
pub const WAKE_PULL_EVERY: std::time::Duration = std::time::Duration::from_secs(120);

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// The user key from its 32-byte seed.
fn user_from_seed(seed: &[u8]) -> Result<Keypair> {
    let mut bytes: [u8; 32] = seed.try_into().map_err(|_| anyhow!("the identity seed is not 32 bytes"))?;
    let key = Keypair::ed25519_from_bytes(&mut bytes).map_err(|e| anyhow!("identity seed: {e}"))?;
    Ok(key)
}

/// Why a record couldn't be written.
#[derive(Debug)]
pub enum WriteError {
    /// Sealing needs an epoch key and this device hasn't got one yet (a sync pass brings it).
    NoKeyYet,
    Other(anyhow::Error),
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::NoKeyYet => write!(f, "waiting for the first key from your other devices"),
            WriteError::Other(e) => write!(f, "{e:#}"),
        }
    }
}

/// Reloads the op log saved by [`Identity::save`], so our sequence numbers carry on and history
/// survives a restart.
fn load_log(dir: &Path, st: &mut SyncState) {
    let Ok(bytes) = std::fs::read(dir.join("log.json")) else { return };
    let Ok(entries) = serde_json::from_slice::<Vec<LogEntry>>(&bytes) else {
        tracing::warn!("log.json is unreadable; starting the op log empty");
        return;
    };
    let mut by_node: std::collections::BTreeMap<String, Vec<LogEntry>> = Default::default();
    for e in entries {
        by_node.entry(e.node_id.0.clone()).or_default().push(e);
    }
    for (node, ops) in by_node {
        let (from_seq, to_seq) = (ops.iter().map(|e| e.seq).min().unwrap_or(0), ops.iter().map(|e| e.seq).max().unwrap_or(0));
        let node_id = weirdnet_sync::NodeId::new(node);
        st.log.integrate(&LogPage { node_id: node_id.clone(), from_seq, to_seq, ops });
        let c = st.log.contiguous(&node_id);
        st.log.set_cursor(&node_id, c);
    }
}

pub(crate) fn write_private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_private(path, bytes).map_err(std::io::Error::other)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    // A temp name of its own for every write, so two saves at once never rename each other's file.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = path.with_file_name(format!(".{name}.{}.{n}.tmp", std::process::id()));
    let r = write_then_rename(&tmp, path, bytes);
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

fn write_then_rename(tmp: &Path, path: &Path, bytes: &[u8]) -> Result<()> {
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(tmp).with_context(|| format!("writing {}", tmp.display()))?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(tmp, path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn ensure_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

impl Identity {
    fn wake_pull_due(&self, peer: &str) -> bool {
        self.wake_pulled.lock().map(|m| m.get(peer).is_none_or(|t| t.elapsed() >= WAKE_PULL_EVERY)).unwrap_or(true)
    }

    /// The next sync pass pulls every device's wake cards (after this device's wake settings
    /// change, say).
    pub fn refresh_wake_cards(&self) {
        if let Ok(mut m) = self.wake_pulled.lock() {
            m.clear();
        }
    }

    /// Whether `dir` holds an identity.
    pub fn exists(dir: &Path) -> bool {
        dir.join("seed").is_file() && dir.join("node.key").is_file()
    }

    /// A new Weirdnet user identity with this device as its first node. For tests and the demo;
    /// a real device joins the user's identity through a device link.
    pub fn create(dir: &Path, label: &str) -> Result<Self> {
        if Self::exists(dir) {
            bail!("{} already holds an identity", dir.display());
        }
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|e| anyhow!("random seed: {e}"))?;
        let user = user_from_seed(&seed)?;
        let node_key = Keypair::generate_ed25519();
        let cert = Roster::self_certify(&WEIRDNET, &user, &node_key.public().to_peer_id().to_string(), label, now_ms())
            .map_err(|e| anyhow!("certifying this node: {e}"))?;
        let mut roster = Roster::new();
        roster.upsert(cert.clone());
        let m = Self::build(dir, seed.to_vec(), node_key, cert, roster, EpochKeys::default())?;
        m.save()?;
        Ok(m)
    }

    /// The identity a device link delivered: `seed`, the node certificate the existing device
    /// made for `node_key`, and its roster.
    pub fn adopt(dir: &Path, node_key: Keypair, seed: &[u8], cert: NodeCert, settings: &str) -> Result<Self> {
        if Self::exists(dir) {
            bail!("{} already holds an identity", dir.display());
        }
        let user = user_from_seed(seed)?;
        let peer = node_key.public().to_peer_id().to_string();
        if cert.peer_id != peer || cert.user_id != weirdnet_net::user::user_id(&user.public()) || !WEIRDNET.verify_cert(&user.public(), &cert) {
            bail!("the link's certificate is not for this node and identity");
        }
        let given: LinkSettings =
            if settings.trim().is_empty() { LinkSettings::default() } else { serde_json::from_str(settings).unwrap_or_default() };
        let mut roster = Roster::new();
        roster.merge(&given.roster, &user.public(), &WEIRDNET);
        roster.upsert(cert.clone());
        let m = Self::build(dir, seed.to_vec(), node_key, cert, roster, EpochKeys::default())?;
        m.save()?;
        Ok(m)
    }

    /// The identity stored in `dir`, if any.
    pub fn open(dir: &Path) -> Result<Option<Self>> {
        if !Self::exists(dir) {
            return Ok(None);
        }
        let seed = std::fs::read(dir.join("seed")).context("reading the identity seed")?;
        let node_key = Keypair::from_protobuf_encoding(&std::fs::read(dir.join("node.key")).context("reading the node key")?)
            .map_err(|e| anyhow!("node key: {e}"))?;
        let roster: Roster = match std::fs::read(dir.join("roster.json")) {
            Ok(b) => serde_json::from_slice(&b).context("reading roster.json")?,
            Err(_) => Roster::new(),
        };
        let keys = std::fs::read(dir.join("keys")).ok().and_then(|b| EpochKeys::import(&b).ok()).unwrap_or_default();
        let peer = node_key.public().to_peer_id().to_string();
        let cert = roster.get(&peer).cloned().ok_or_else(|| anyhow!("the roster has no certificate for this node"))?;
        Self::build(dir, seed, node_key, cert, roster, keys).map(Some)
    }

    fn build(dir: &Path, seed: Vec<u8>, node_key: Keypair, our_cert: NodeCert, roster: Roster, keys: EpochKeys) -> Result<Self> {
        ensure_dir(dir)?;
        let user = user_from_seed(&seed)?;
        let mut st = SyncState {
            user_id: our_cert.user_id.clone(),
            user_key: user.public(),
            our_node: weirdnet_sync::NodeId::new(our_cert.peer_id.clone()),
            our_cert,
            seed: Some(seed),
            types: vec![SYNC_TYPE.to_string()],
            roster,
            log: weirdnet_sync::OpLog::new(),
            clock: None,
            device: Device::from_node_key(&node_key).ok(),
            keys,
            auto_first_rotation: true,
            records: Default::default(),
            voter: None,
        };
        st.configure(CONTEXT_TYPE, RecordOptions { sealed: true, ..Default::default() }).map_err(|e| anyhow!("context records: {e}"))?;
        st.configure(crate::wake::WAKE_TYPE, RecordOptions { sealed: true, ..Default::default() }).map_err(|e| anyhow!("wake records: {e}"))?;
        load_log(dir, &mut st);
        st.publish_device_key(now_ms()).map_err(|e| anyhow!("device key: {e}"))?;
        st.refresh_keys();
        Ok(Self { dir: dir.to_path_buf(), node_key, state: Arc::new(Mutex::new(st)), wake_pulled: Mutex::default() })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SyncState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Writes the roster and keys (and, the first time, the seed and node key).
    pub fn save(&self) -> Result<()> {
        ensure_dir(&self.dir)?;
        let (seed, roster, keys, log) = {
            let st = self.lock();
            let entries: Vec<&LogEntry> = st.log.iter().collect();
            (st.seed.clone(), serde_json::to_vec_pretty(&st.roster)?, st.keys.export(), serde_json::to_vec(&entries)?)
        };
        if let Some(seed) = &seed
            && !self.dir.join("seed").is_file()
        {
            write_private(&self.dir.join("seed"), seed)?;
        }
        if !self.dir.join("node.key").is_file() {
            write_private(&self.dir.join("node.key"), &self.node_key.to_protobuf_encoding().map_err(|e| anyhow!("node key: {e}"))?)?;
        }
        write_private(&self.dir.join("roster.json"), &roster)?;
        write_private(&self.dir.join("keys"), &keys)?;
        write_private(&self.dir.join("log.json"), &log)?;
        Ok(())
    }

    /// Writes one record of `typ` (sealed for [`CONTEXT_TYPE`]) and saves. With other devices
    /// the first sealed write has to wait for the first key rotation, which a sync pass brings:
    /// that is an error here with [`WriteError::NoKeyYet`], and the caller keeps the write and
    /// tries again later.
    pub fn write_record(&self, typ: &str, record_id: &str, value: &[u8]) -> std::result::Result<(), WriteError> {
        {
            let mut st = self.lock();
            let mut r = st.write(now_ms() as u64, typ, record_id, value);
            if matches!(&r, Err(weirdnet_sync::SyncError::Refused(m)) if m == "no-epoch-key") {
                // A lone device starts the key chain itself.
                if st.roster.active().count() == 1 {
                    st.rotate(now_ms()).map_err(|e| WriteError::Other(anyhow!("starting the key chain: {e}")))?;
                    r = st.write(now_ms() as u64, typ, record_id, value);
                } else {
                    return Err(WriteError::NoKeyYet);
                }
            }
            r.map_err(|e| WriteError::Other(anyhow!("writing {typ}/{record_id}: {e}")))?;
        }
        self.save().map_err(WriteError::Other)
    }

    /// The latest value of every record of `typ` this device can read, by record id.
    pub fn records(&self, typ: &str) -> Vec<(String, Vec<u8>)> {
        let st = self.lock();
        st.latest_entries(typ)
            .into_iter()
            .filter_map(|e| match st.read(e) {
                Ok(RecordValue::Ready(v)) => Some((e.record_id.clone(), v.to_vec())),
                _ => None,
            })
            .collect()
    }

    /// Rewrites the op log without the values of `typ` nobody can see any more: every entry that is
    /// not its record's latest, the latest of the records `gone` names, and chunks of `typ` that no
    /// kept entry points at. Their slots stay (see [`COMPACTED_TYPE`]); `log.json` is rewritten.
    /// Returns how many entries were dropped. Only this device's copy: every Bragi device compacts
    /// its own after a sync pass.
    ///
    /// Generic: belongs in `weirdnet-sync` next to tombstone GC (noted for the Weirdnet lead).
    pub fn compact(&self, typ: &str, gone: &dyn Fn(&str) -> bool) -> Result<usize> {
        use std::collections::HashSet;
        let dropped = {
            let mut st = self.lock();
            let keep: HashSet<(String, u64)> =
                st.latest_entries(typ).into_iter().filter(|e| !gone(&e.record_id)).map(|e| (e.node_id.0.clone(), e.seq)).collect();
            let ctyp = weirdnet_sync::chunk_type(typ);
            let chunks: HashSet<String> = st
                .log
                .iter()
                .filter(|e| keep.contains(&(e.node_id.0.clone(), e.seq)))
                .filter_map(|e| e.value.as_ref()?.blob.as_ref())
                .flat_map(|b| b.chunk_hashes().map(|h| h.iter().map(|x| format!("{x:02x}")).collect::<String>()).collect::<Vec<_>>())
                .collect();
            let mut dropped = 0;
            let entries: Vec<LogEntry> = st
                .log
                .iter()
                .map(|e| {
                    let stale = (e.typ == typ && !keep.contains(&(e.node_id.0.clone(), e.seq))) || (e.typ == ctyp && !chunks.contains(&e.record_id));
                    if !stale {
                        return e.clone();
                    }
                    dropped += 1;
                    LogEntry {
                        node_id: e.node_id.clone(),
                        seq: e.seq,
                        hlc: e.hlc.clone(),
                        typ: COMPACTED_TYPE.into(),
                        record_id: String::new(),
                        op: Vec::new(),
                        value: None,
                    }
                })
                .collect();
            if dropped == 0 {
                return Ok(0);
            }
            let (cursors, type_cursors) = (st.log.cursors(), st.log.type_cursors());
            let mut log = weirdnet_sync::OpLog::new();
            let mut run: Vec<LogEntry> = Vec::new();
            let fold = |log: &mut weirdnet_sync::OpLog, run: Vec<LogEntry>| {
                if let (Some(first), Some(last)) = (run.first(), run.last()) {
                    let (node_id, from_seq, to_seq) = (first.node_id.clone(), first.seq, last.seq);
                    log.integrate(&LogPage { node_id, from_seq, to_seq, ops: run });
                }
            };
            for e in entries {
                if run.first().is_some_and(|f| f.node_id != e.node_id) {
                    fold(&mut log, std::mem::take(&mut run));
                }
                run.push(e);
            }
            fold(&mut log, run);
            for (node, seq) in cursors {
                log.set_cursor(&node, seq);
            }
            for ((node, t), seq) in type_cursors {
                log.set_cursor_for(&node, &t, seq);
            }
            st.log = log;
            dropped
        };
        self.save()?;
        Ok(dropped)
    }

    /// The directory this identity is stored in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn node_key(&self) -> &Keypair {
        &self.node_key
    }

    pub fn peer_id(&self) -> PeerId {
        self.node_key.public().to_peer_id()
    }

    pub fn state(&self) -> Arc<Mutex<SyncState>> {
        self.state.clone()
    }

    pub fn user_key(&self) -> PublicKey {
        self.lock().user_key.clone()
    }

    pub fn user_id(&self) -> String {
        self.lock().user_id.clone()
    }

    pub fn our_cert(&self) -> NodeCert {
        self.lock().our_cert.clone()
    }

    pub fn roster(&self) -> Roster {
        self.lock().roster.clone()
    }

    /// The roster as it is now, for the agent's `RosterAuth` and `LinkedNodes`.
    pub fn roster_view(&self) -> Arc<dyn Fn() -> Roster + Send + Sync> {
        let st = self.state.clone();
        Arc::new(move || st.lock().unwrap_or_else(|e| e.into_inner()).roster.clone())
    }

    /// The transfer payload for a device linked from here.
    pub fn link_settings(&self) -> String {
        self.link_settings_with(Vec::new())
    }

    /// [`link_settings`](Self::link_settings) with `peers`: where the new device can dial the
    /// user's other devices.
    pub fn link_settings_with(&self, peers: Vec<String>) -> String {
        serde_json::to_string(&LinkSettings { roster: self.roster(), peers }).unwrap_or_else(|_| "{}".into())
    }

    /// The seed, for a device link sent from here.
    pub fn seed(&self) -> Result<weirdnet_net::Key32> {
        let st = self.lock();
        let seed = st.seed.as_ref().ok_or_else(|| anyhow!("this device holds no identity seed"))?;
        weirdnet_net::Key32::from_slice(seed).map_err(|e| anyhow!("identity seed: {e}"))
    }

    /// The user keypair (from the seed), to certify a server linked cert-only from here.
    pub fn user_keypair(&self) -> Result<Keypair> {
        let st = self.lock();
        let seed = st.seed.as_ref().ok_or_else(|| anyhow!("this device holds no identity seed"))?;
        user_from_seed(seed)
    }

    /// The sync bundle for a server linked cert-only from here (`weirdnet_sync::LinkBundle`
    /// bytes): signs its service grant with `roles` and the sync `types` it may hold keys for
    /// (the grant joins this roster too; with types and no key chain yet this rotates first).
    /// Saves.
    pub fn service_bundle(&self, peer: &str, roles: &[String], types: &[String]) -> Result<zeroize::Zeroizing<Vec<u8>>> {
        let b = self.lock().service_bundle(peer, roles, types, now_ms()).map_err(|e| anyhow!("the server's grant: {e}"))?;
        self.save()?;
        b.to_bytes().map_err(|e| anyhow!("the sync bundle: {e}"))
    }

    /// Moves conversation records to the key the devices that share them now can open
    /// (`weirdnet_sync` 0.8.4 `reseal_stale`): once a server is linked cert-only with
    /// [`CONTEXT_TYPE`] in its grant, records written before the link move from the epoch key
    /// to the type-scoped key it holds; once no such server is left they move back, so
    /// devices on older Bragi releases read them again. Run it after a sync pass (the new
    /// entry is a later write of the same value). A service device, or one with no key chain
    /// yet, has nothing to do. Returns how many records moved; saves when any did.
    pub fn reseal_context(&self) -> Result<usize> {
        let n = {
            let mut st = self.lock();
            if st.is_service() {
                return Ok(0);
            }
            match st.reseal_stale(now_ms() as u64, CONTEXT_TYPE) {
                Ok(ids) => ids.len(),
                Err(weirdnet_sync::SyncError::Refused(m)) if m == "no-epoch-key" => 0,
                Err(e) => bail!("{e}"),
            }
        };
        if n > 0 {
            self.save()?;
        }
        Ok(n)
    }

    /// Whether `peer`'s signed grant makes it a service device (a server linked without the seed).
    pub fn is_service(&self, peer: &str) -> bool {
        self.lock().roster.is_service(peer)
    }

    /// The user linked `cert`'s device from here: it joins the roster (and the key chain once
    /// its device key is known).
    pub fn approve(&self, cert: NodeCert) -> Result<()> {
        self.lock().approve(cert, now_ms()).map_err(|e| anyhow!("approving the device: {e}"))?;
        self.save()
    }

    /// Revokes a device everywhere this roster reaches.
    pub fn revoke(&self, peer: &str) -> Result<NodeCert> {
        let cert = {
            let mut st = self.lock();
            if peer == st.our_cert.peer_id {
                bail!("this is this node's own certificate");
            }
            let seed = st.seed.as_ref().ok_or_else(|| anyhow!("this device holds no identity seed"))?;
            let user = user_from_seed(seed)?;
            st.revoke(&user, peer, now_ms()).map_err(|e| anyhow!("revoking {peer}: {e}"))?
        };
        self.save()?;
        Ok(cert)
    }

    /// One sync pass against every other active device in the roster (blocking; call from a
    /// blocking task). `addrs` gives the addresses to dial a device at. Returns the devices
    /// reached.
    pub fn tick(&self, node: &SyncNode, addrs: &dyn Fn(&NodeCert) -> Vec<Multiaddr>) -> usize {
        let me = node.peer_id().to_string();
        let peers: Vec<NodeCert> = self.lock().roster.active().filter(|c| c.peer_id != me).cloned().collect();
        let mut reached = 0;
        let mut context_ok = true;
        for cert in peers {
            let Ok(peer) = PeerId::from_str(&cert.peer_id) else { continue };
            let a = addrs(&cert);
            // One session for every type (weirdnet-sync 0.8.5 `pull_types_shared`: one Hello and
            // roster catch-up), pulled into a snapshot and merged back, so our handler can answer a
            // peer pulling from us at the same moment. Wake cards only between the user's full
            // devices (no server is granted them).
            let wake = !self.is_service(&cert.peer_id) && self.wake_pull_due(&cert.peer_id);
            let mut types = vec![SYNC_TYPE, CONTEXT_TYPE];
            if wake {
                types.push(crate::wake::WAKE_TYPE);
            }
            let outs = weirdnet_sync::pull_types_shared(&self.state, node, peer, &a, &types);
            let error = |t: &str| outs.types.iter().find(|(n, _)| n == t).and_then(|(_, o)| o.error.as_ref());
            let applied = |t: &str| outs.types.iter().find(|(n, _)| n == t).map_or(0, |(_, o)| o.applied);
            match error(SYNC_TYPE) {
                None => reached += 1,
                Some(e) => {
                    context_ok = false;
                    tracing::debug!(peer = %cert.peer_id, applied = applied(SYNC_TYPE), "sync pass: {e}");
                    continue;
                }
            }
            if let Some(e) = error(CONTEXT_TYPE) {
                context_ok = false;
                tracing::debug!(peer = %cert.peer_id, applied = applied(CONTEXT_TYPE), "context pass: {e}");
            }
            if wake {
                match error(crate::wake::WAKE_TYPE) {
                    None => {
                        if let Ok(mut m) = self.wake_pulled.lock() {
                            m.insert(cert.peer_id.clone(), std::time::Instant::now());
                        }
                    }
                    Some(e) => tracing::debug!(peer = %cert.peer_id, applied = applied(crate::wake::WAKE_TYPE), "wake pass: {e}"),
                }
            }
        }
        // Only after a pass that reached every device, so the re-sealed copy is the latest.
        if context_ok {
            match self.reseal_context() {
                Ok(0) => {}
                Ok(n) => tracing::info!("re-sealed {n} conversation records for the devices that share them now"),
                Err(e) => tracing::warn!("re-sealing conversations: {e}"),
            }
        }
        if let Err(e) = self.save() {
            tracing::warn!("saving the roster: {e}");
        }
        reached
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use weirdnet_sync::CallerCheck;

    #[test]
    fn concurrent_writes_to_one_file_all_succeed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("roster.json");
        std::thread::scope(|s| {
            for i in 0..8u8 {
                let path = &path;
                s.spawn(move || {
                    for _ in 0..50 {
                        write_private(path, &[i; 64]).unwrap();
                    }
                });
            }
        });
        let b = std::fs::read(&path).unwrap();
        assert!(b.len() == 64 && b.iter().all(|x| *x == b[0]), "one whole write wins");
        let left: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("roster.json")], "no temp files left");
    }

    #[test]
    fn create_open_link_and_revoke() {
        let tmp = tempfile::tempdir().unwrap();
        let a_dir = tmp.path().join("a");
        let a = Identity::create(&a_dir, "box-a").unwrap();
        assert!(Identity::create(&a_dir, "again").is_err(), "never overwrites an identity");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(a_dir.join("seed")).unwrap().permissions().mode() & 0o777, 0o600);
        }

        // Link a second device the way device-link does: a cert from the seed holder.
        let b_key = Keypair::generate_ed25519();
        let user = user_from_seed(&a.seed().unwrap().0).unwrap();
        let b_cert = Roster::self_certify(&WEIRDNET, &user, &b_key.public().to_peer_id().to_string(), "box-b", now_ms()).unwrap();
        a.approve(b_cert.clone()).unwrap();
        let (seed, settings) = (a.seed().unwrap(), a.link_settings());
        let b = Identity::adopt(&tmp.path().join("b"), b_key, &seed.0, b_cert.clone(), &settings).unwrap();
        assert_eq!(b.user_id(), a.user_id());
        let a_peer = a.peer_id().to_string();
        let b_peer = b.peer_id().to_string();
        assert_eq!(b.roster().check_caller(&WEIRDNET, &b.user_key(), &a_peer, None), CallerCheck::Linked);
        assert_eq!(a.roster().check_caller(&WEIRDNET, &a.user_key(), &b_peer, Some(&b_cert)), CallerCheck::Linked);

        // A stranger with another identity is not admitted.
        let c = Identity::create(&tmp.path().join("c"), "stranger").unwrap();
        assert_eq!(a.roster().check_caller(&WEIRDNET, &a.user_key(), &c.peer_id().to_string(), Some(&c.our_cert())), CallerCheck::WrongUser);

        // Reopening keeps the roster; revoking sticks.
        let again = Identity::open(&a_dir).unwrap().unwrap();
        assert_eq!(again.peer_id(), a.peer_id());
        again.revoke(&b_peer).unwrap();
        let reopened = Identity::open(&a_dir).unwrap().unwrap();
        assert_eq!(reopened.roster().check_caller(&WEIRDNET, &reopened.user_key(), &b_peer, None), CallerCheck::Revoked);
        assert!(reopened.revoke(&a_peer).is_err());
    }

    /// A conversation written before an inference server was linked cert-only stays on the
    /// epoch key, which the server never holds, until `reseal_context` moves it; one written
    /// after the link is readable there at once. Revoking the server moves them back.
    #[test]
    fn conversations_from_before_a_server_link_are_resealed_for_it() {
        let tmp = tempfile::tempdir().unwrap();
        let a = Identity::create(&tmp.path().join("a"), "box-a").unwrap();
        a.write_record(CONTEXT_TYPE, "c1/meta", b"before the link").unwrap();
        assert_eq!(a.reseal_context().unwrap(), 0, "nothing to move without a server");

        let s_key = Keypair::generate_ed25519();
        let s_peer = s_key.public().to_peer_id().to_string();
        let user = user_from_seed(&a.seed().unwrap().0).unwrap();
        let s_cert = Roster::self_certify(&WEIRDNET, &user, &s_peer, "ymir", now_ms()).unwrap();
        let types = vec!["ymir.pool".to_string(), CONTEXT_TYPE.to_string()];
        let bundle = a.service_bundle(&s_peer, &["ymir.inference".to_string()], &types).unwrap();
        a.approve(s_cert.clone()).unwrap();
        let bundle = weirdnet_sync::LinkBundle::from_bytes(&bundle).unwrap();
        let mut svc = SyncState::new_service(a.user_key(), s_cert, &s_key, vec![CONTEXT_TYPE.to_string()], &bundle).unwrap();
        svc.configure(CONTEXT_TYPE, RecordOptions { sealed: true, ..Default::default() }).unwrap();
        a.write_record(CONTEXT_TYPE, "c2/meta", b"after the link").unwrap();

        let opens = |svc: &mut SyncState, rid: &str| {
            svc.log = a.lock().log.clone();
            let e = svc.latest_entries(CONTEXT_TYPE).into_iter().find(|e| e.record_id == rid).cloned().unwrap();
            matches!(svc.read(&e), Ok(RecordValue::Ready(_)))
        };
        assert!(opens(&mut svc, "c2/meta"), "written after the link: readable on the server");
        assert!(!opens(&mut svc, "c1/meta"), "written before the link: still on the epoch key");

        assert_eq!(a.reseal_context().unwrap(), 1);
        assert!(opens(&mut svc, "c1/meta"), "re-sealed for the server");
        assert_eq!(a.reseal_context().unwrap(), 0, "nothing left to move");
        let recs: BTreeMap<String, Vec<u8>> = a.records(CONTEXT_TYPE).into_iter().collect();
        assert_eq!(recs["c1/meta"], b"before the link");
        assert_eq!(recs["c2/meta"], b"after the link");

        // With the server revoked, both move back to the epoch key for older devices.
        a.revoke(&s_peer).unwrap();
        assert_eq!(a.reseal_context().unwrap(), 2);
        let recs: BTreeMap<String, Vec<u8>> = a.records(CONTEXT_TYPE).into_iter().collect();
        assert_eq!(recs.len(), 2);
    }
}
