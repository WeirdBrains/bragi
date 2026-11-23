//! The desktop side of the link (`bragi-tui linker`): a Weirdnet node of its own that serves the
//! code tools to paired phones as app calls (`bragi-link`), over the LAN or VPN directly or
//! through a Weirdnet relay. Noise authenticates both ends on every connection and encrypts end
//! to end, so a relay only forwards bytes it can't read or change.
//!
//! A call is answered only when the caller's peer id (checked by Noise) and its bearer token both
//! match one paired device. Pairing takes a one-time secret from a QR code (or a typed code) the
//! desktop shows, which expires after a few minutes, after one use, or after a few wrong tries.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, SystemTime};

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use weirdnet_net::{AppError, AppRequest, AppResponse, Keypair, Multiaddr, Node, NodeConfig, PeerId};

use crate::core::lock;
use crate::error::{CoreError, ErrorKind, Result};
use crate::history::now_ms;

use super::confine::Root;
use super::proto::*;
use super::workspace;

/// The port the link listens on unless told otherwise (TCP and QUIC).
pub const DEFAULT_PORT: u16 = 47480;
/// How long a pairing code works.
pub const OFFER_TTL: Duration = Duration::from_secs(5 * 60);
/// Wrong secrets before a pairing code is cancelled.
pub const MAX_TRIES: u32 = 5;
/// The most a request body may hold.
const BODY_MAX: u64 = 32 * 1024 * 1024;

pub(crate) type LinkNode = Node<Vec<u8>, Vec<u8>>;

/// The link uses app calls only; plain requests get an empty answer.
pub(crate) struct NoRequests;

impl weirdnet_net::Handler<Vec<u8>, Vec<u8>> for NoRequests {
    fn request(&self, _from: &PeerId, _req: Vec<u8>) -> Vec<u8> {
        Vec::new()
    }
}

pub(crate) fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    let _ = getrandom::fill(&mut b);
    b
}

/// The link's folder in a Bragi profile.
pub fn link_dir(profile: &Path) -> PathBuf {
    profile.join("linker")
}

/// Makes `dir` readable by this user only.
pub(crate) fn private_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub(crate) fn load_or_make_key(path: &Path) -> Result<Keypair> {
    if let Ok(b) = std::fs::read(path) {
        return Keypair::from_protobuf_encoding(&b).map_err(|e| CoreError::new(ErrorKind::Storage, format!("{}: {e}", path.display())));
    }
    let k = Keypair::generate_ed25519();
    if let Some(p) = path.parent() {
        private_dir(p)?;
    }
    let bytes = k.to_protobuf_encoding().map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?;
    crate::identity::write_private_file(path, &bytes)?;
    Ok(k)
}

fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<()> {
    if let Some(p) = path.parent() {
        private_dir(p)?;
    }
    let bytes = serde_json::to_vec_pretty(v).map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?;
    crate::identity::write_private_file(path, &bytes)?;
    Ok(())
}

// ---- Settings and devices (files, so `linker revoke` works while a server runs) -----------

/// What the desktop shares: `<profile>/linker/settings.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// The project roots a phone may use. Nothing outside them is reachable.
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    /// Shell commands from a phone wait for a yes on the desktop.
    #[serde(default)]
    pub confirm_shell: bool,
}

impl Settings {
    pub fn load(dir: &Path) -> Self {
        std::fs::read(dir.join("settings.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        write_json(&dir.join("settings.json"), self)
    }

    /// Adds a root (canonicalized; it must be a folder). Returns whether it was new.
    pub fn add_root(&mut self, dir: &Path) -> Result<bool> {
        let r = Root::new(dir)?;
        if self.roots.iter().any(|p| p == r.path()) {
            return Ok(false);
        }
        self.roots.push(r.path().to_path_buf());
        Ok(true)
    }

    pub fn remove_root(&mut self, dir: &Path) -> bool {
        let canon = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        let n = self.roots.len();
        self.roots.retain(|p| *p != canon && p != dir);
        n != self.roots.len()
    }
}

/// One paired phone, as stored (`devices.json`, 0600). The token itself is never stored, only
/// its BLAKE3 hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Device {
    id: String,
    label: String,
    /// The phone's link peer id: calls must come from this key.
    peer: String,
    token_hash: String,
    paired_ms: i64,
    #[serde(default)]
    last_seen_ms: i64,
}

/// A paired phone, for listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceView {
    pub id: String,
    pub label: String,
    pub peer: String,
    pub paired_ms: i64,
    pub last_seen_ms: i64,
}

impl From<&Device> for DeviceView {
    fn from(d: &Device) -> Self {
        DeviceView { id: d.id.clone(), label: d.label.clone(), peer: d.peer.clone(), paired_ms: d.paired_ms, last_seen_ms: d.last_seen_ms }
    }
}

fn load_devices(dir: &Path) -> Vec<Device> {
    std::fs::read(dir.join("devices.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// The phones paired with this desktop.
pub fn devices(dir: &Path) -> Vec<DeviceView> {
    load_devices(dir).iter().map(DeviceView::from).collect()
}

/// Unpairs a phone (by id, label or peer id); a running server refuses it from its next call.
pub fn revoke(dir: &Path, which: &str) -> Result<Vec<DeviceView>> {
    let mut all = load_devices(dir);
    let (gone, kept): (Vec<Device>, Vec<Device>) = all.drain(..).partition(|d| d.id == which || d.label == which || d.peer == which);
    if gone.is_empty() {
        return Err(CoreError::new(ErrorKind::NotFound, format!("no paired device `{which}`")));
    }
    write_json(&dir.join("devices.json"), &kept)?;
    Ok(gone.iter().map(DeviceView::from).collect())
}

/// A file read again only when it changed on disk.
struct Cached<T> {
    path: PathBuf,
    seen: Option<SystemTime>,
    value: T,
    load: fn(&Path) -> T,
}

impl<T> Cached<T> {
    fn new(dir: &Path, name: &str, load: fn(&Path) -> T) -> Self {
        Cached { path: dir.join(name), seen: None, value: load(dir), load }
    }

    fn get(&mut self) -> &mut T {
        let m = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if m != self.seen {
            self.seen = m;
            if let Some(dir) = self.path.parent() {
                self.value = (self.load)(dir);
            }
        }
        &mut self.value
    }

    /// After writing the file ourselves.
    fn wrote(&mut self) {
        self.seen = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
    }
}

// ---- The server -------------------------------------------------------------------------

/// How to run the server.
#[derive(Debug, Clone)]
pub struct ServerOptions {
    /// `<profile>/linker`.
    pub dir: PathBuf,
    /// What phones call this desktop (`Bragi on dev-box`).
    pub label: String,
    /// The in-memory transport (tests).
    pub memory: bool,
    pub port: u16,
    /// Hold a reservation on the Weirdnet relays, so phones off the LAN reach it.
    pub weirdnet: bool,
    /// Listen on every interface, not only LAN and VPN ones.
    pub any_interface: bool,
}

impl ServerOptions {
    pub fn new(dir: impl Into<PathBuf>, label: impl Into<String>) -> Self {
        ServerOptions { dir: dir.into(), label: label.into(), memory: false, port: DEFAULT_PORT, weirdnet: true, any_interface: false }
    }
}

/// A shell command a phone wants to run, for the desktop's yes or no.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellAsk {
    pub device: String,
    pub root: PathBuf,
    pub cwd: String,
    pub command: String,
}

/// Answers a [`ShellAsk`]: `true` runs it.
pub type Confirm = Arc<dyn Fn(ShellAsk) -> BoxFuture<'static, bool> + Send + Sync>;

/// What happened, for the desktop to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEvent {
    Paired(DeviceView),
    /// A pairing try or call that was turned away, and why.
    Refused(String),
    Shell {
        device: String,
        command: String,
    },
    Wrote {
        device: String,
        files: Vec<String>,
    },
    Unpaired(DeviceView),
}

/// A pairing code on show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub link: PairLink,
    /// The QR code's text.
    pub url: String,
    /// The code to type instead, when the desktop has a LAN or VPN IPv4 address.
    pub short: Option<String>,
    pub expires_ms: i64,
}

struct Pending {
    secret: [u8; 16],
    short: [u8; 6],
    expires_ms: i64,
    tries: u32,
}

struct Shared {
    dir: PathBuf,
    label: String,
    weirdnet: bool,
    memory: bool,
    settings: Mutex<Cached<Settings>>,
    devices: Mutex<Cached<Vec<Device>>>,
    pending: Mutex<Option<Pending>>,
    events: broadcast::Sender<LinkEvent>,
    confirm: Option<Confirm>,
    node: OnceLock<Weak<LinkNode>>,
    last_saved: Mutex<i64>,
}

pub struct LinkServer {
    shared: Arc<Shared>,
    node: Arc<LinkNode>,
}

/// LAN and VPN addresses: private IPv4 (10/8, 172.16/12, 192.168/16), shared CGNAT space
/// (100.64/10, which Tailscale and similar VPNs use) and IPv6 unique-local (fc00::/7).
pub fn is_lan(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xfe00) == 0xfc00,
    }
}

fn listen_addrs(port: u16, any: bool) -> Vec<Multiaddr> {
    if any {
        return NodeConfig::default_listen(port);
    }
    let mut out = Vec::new();
    for iface in if_addrs::get_if_addrs().unwrap_or_default() {
        let ip = iface.ip();
        if iface.is_loopback() || !is_lan(&ip) || is_virtual(&iface.name) {
            continue;
        }
        let (proto, host) = match ip {
            IpAddr::V4(v) => ("ip4", v.to_string()),
            IpAddr::V6(v) => ("ip6", v.to_string()),
        };
        for a in [format!("/{proto}/{host}/tcp/{port}"), format!("/{proto}/{host}/udp/{port}/quic-v1")] {
            if let Ok(m) = a.parse() {
                out.push(m);
            }
        }
    }
    out
}

/// Container and VM bridges: private addresses no phone can reach.
fn is_virtual(name: &str) -> bool {
    ["docker", "br-", "veth", "virbr", "lxc", "lxd", "cni", "flannel", "podman", "vmnet", "vboxnet"].iter().any(|p| name.starts_with(p))
}

/// Which direct address a phone should try first: home LAN, then other private ranges, then
/// VPN (100.64/10), IPv6 last; TCP before QUIC.
fn rank(addr: &str) -> (u8, u8) {
    let ip = addr.split('/').nth(2).and_then(|h| h.parse::<IpAddr>().ok());
    let net = match ip {
        Some(IpAddr::V4(v)) if v.octets()[0] == 192 => 0,
        Some(IpAddr::V4(v)) if v.octets()[0] == 10 => 1,
        Some(IpAddr::V4(v)) if v.octets()[0] == 172 => 2,
        Some(IpAddr::V4(_)) => 3,
        _ => 4,
    };
    (net, if addr.contains("/tcp/") { 0 } else { 1 })
}

fn relay_peer(relay: &str) -> &str {
    relay.rsplit("/p2p/").next().unwrap_or("")
}

impl LinkServer {
    /// Starts the link node and serves the link. `confirm` answers shell commands when
    /// [`Settings::confirm_shell`] is on (without it, they are refused while that's on).
    pub fn start(opts: ServerOptions, confirm: Option<Confirm>) -> Result<Self> {
        private_dir(&opts.dir)?;
        let key = load_or_make_key(&opts.dir.join("node.key"))?;
        let mut cfg = if opts.memory { weirdnet_sync::node_config(true) } else { NodeConfig::default() };
        if !opts.memory {
            cfg.listen = listen_addrs(opts.port, opts.any_interface);
            if cfg.listen.is_empty() && !opts.weirdnet {
                return Err(CoreError::new(ErrorKind::Unavailable, "no LAN or VPN address to listen on; use --any-interface or turn Weirdnet on"));
            }
            cfg.mdns = false;
            cfg.discovery = weirdnet_net::DiscoveryMode::Disabled;
            if opts.weirdnet {
                cfg.relays = weirdnet_device_link::KNOWN_RELAYS.iter().filter_map(|r| r.parse().ok()).collect();
            }
        }
        cfg.label = opts.label.clone();
        cfg.version = format!("bragi-link/{}", env!("CARGO_PKG_VERSION"));
        cfg.app_body_max = BODY_MAX;
        cfg.app_idle_timeout = Duration::from_secs(120);
        let node = Arc::new(
            LinkNode::start(&weirdnet_net::WEIRDNET, key, cfg, Arc::new(NoRequests))
                .map_err(|e| CoreError::new(ErrorKind::Unavailable, format!("starting the link (is port {} free? try --port): {e}", opts.port)))?,
        );
        let shared = Arc::new(Shared {
            settings: Mutex::new(Cached::new(&opts.dir, "settings.json", Settings::load)),
            devices: Mutex::new(Cached::new(&opts.dir, "devices.json", load_devices)),
            dir: opts.dir,
            label: opts.label,
            weirdnet: opts.weirdnet && !opts.memory,
            memory: opts.memory,
            pending: Mutex::new(None),
            events: broadcast::channel(64).0,
            confirm,
            node: OnceLock::new(),
            last_saved: Mutex::new(0),
        });
        let _ = shared.node.set(Arc::downgrade(&node));
        let s = shared.clone();
        node.serve_app(
            APP,
            Arc::new(move |peer: PeerId, req: AppRequest| {
                let s = s.clone();
                async move { Ok(s.handle(peer, req).await) }
            }),
        )
        .map_err(|e| CoreError::new(ErrorKind::Other, format!("serving the link: {e}")))?;
        Ok(LinkServer { shared, node })
    }

    pub fn peer_id(&self) -> String {
        self.node.peer_id().to_string()
    }

    /// Every address the link node listens on, with `/p2p/` (the memory address in tests).
    pub fn addresses(&self) -> Vec<String> {
        let id = self.node.peer_id();
        self.node.status().listening.iter().map(|a| if a.contains("/p2p/") { a.clone() } else { format!("{a}/p2p/{id}") }).collect()
    }

    /// Direct addresses to put in a pairing code: LAN/VPN TCP and QUIC, without `/p2p/`.
    pub fn direct(&self) -> Vec<String> {
        self.shared.direct()
    }

    /// Whether a Weirdnet relay holds a reservation for this desktop now.
    pub fn on_weirdnet(&self) -> bool {
        !self.shared.relays_now(true).is_empty()
    }

    pub fn events(&self) -> broadcast::Receiver<LinkEvent> {
        self.shared.events.subscribe()
    }

    /// A new pairing code (replacing any earlier one), good for `ttl` or one pairing.
    pub fn offer(&self, ttl: Duration) -> Offer {
        let secret: [u8; 16] = random_bytes();
        let short: [u8; 6] = random_bytes();
        let expires_ms = now_ms() + ttl.as_millis() as i64;
        *lock(&self.shared.pending) = Some(Pending { secret, short, expires_ms, tries: 0 });
        // TCP only and a few at most, so the QR code stays easy to scan; the phone learns the
        // rest from `hello` once paired.
        let direct: Vec<String> = self.direct().into_iter().filter(|a| !a.contains("/udp/")).take(4).collect();
        let link = PairLink {
            peer: self.peer_id(),
            secret: hex(&secret),
            label: self.shared.label.clone(),
            direct: direct.clone(),
            relays: self.shared.relays_now(false),
            expires_ms,
        };
        let short_code = direct.iter().find_map(|a| {
            let mut parts = a.split('/').skip(1);
            match (parts.next(), parts.next(), parts.next(), parts.next()) {
                (Some("ip4"), Some(ip), Some("tcp"), Some(port)) => {
                    Some(ShortCode { ip: ip.parse().ok()?, port: port.parse().ok()?, secret: short, fingerprint: fingerprint(&self.node.peer_id()) })
                }
                _ => None,
            }
        });
        Offer { url: link.to_url(), link, short: short_code.map(|c| c.encode()), expires_ms }
    }

    /// Ends the pairing code on show, if any.
    pub fn cancel_offer(&self) {
        *lock(&self.shared.pending) = None;
    }

    /// Whether a pairing code is still waiting.
    pub fn offering(&self) -> bool {
        lock(&self.shared.pending).as_ref().is_some_and(|p| p.expires_ms > now_ms())
    }

    pub fn stop(self) {
        self.node.halt();
    }
}

impl Drop for LinkServer {
    fn drop(&mut self) {
        self.node.halt();
    }
}

fn json<T: Serialize>(status: u16, v: &T) -> AppResponse {
    AppResponse::new(status).header("content-type", "application/json").body(serde_json::to_vec(v).unwrap_or_default())
}

fn error(e: &CoreError) -> AppResponse {
    let status = match e.kind {
        ErrorKind::Refused | ErrorKind::NotPaired => 403,
        ErrorKind::Pairing => 401,
        ErrorKind::NotFound => 404,
        ErrorKind::BadRequest => 400,
        ErrorKind::Storage => 409,
        _ => 500,
    };
    json(status, &WireError { kind: e.kind, message: e.message.clone() })
}

fn parse<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T> {
    serde_json::from_slice(body).map_err(|e| CoreError::new(ErrorKind::BadRequest, format!("bad request: {e}")))
}

fn same(a: &[u8], b: &[u8]) -> bool {
    // Hashes compared with BLAKE3's constant-time equality.
    blake3::hash(a) == blake3::hash(b)
}

impl Shared {
    fn emit(&self, e: LinkEvent) {
        let _ = self.events.send(e);
    }

    fn node(&self) -> Option<Arc<LinkNode>> {
        self.node.get().and_then(Weak::upgrade)
    }

    fn direct(&self) -> Vec<String> {
        let Some(node) = self.node() else { return vec![] };
        let mut out = Vec::new();
        for a in node.status().listening {
            if is_circuit(&a) {
                continue;
            }
            let plain = a.split("/p2p/").next().unwrap_or(&a).to_string();
            if plain.starts_with("/memory") {
                // Tests only: the in-memory transport has no IP.
                if self.memory && !out.contains(&plain) {
                    out.push(plain);
                }
                continue;
            }
            let ip = plain.split('/').nth(2).and_then(|h| h.parse::<IpAddr>().ok());
            if ip.is_some_and(|ip| !ip.is_loopback() && !ip.is_unspecified()) && !out.contains(&plain) {
                out.push(plain);
            }
        }
        out.sort_by_key(|a| rank(a));
        out
    }

    /// The known relays this desktop is on (`strict`), or that it tries to be on.
    fn relays_now(&self, strict: bool) -> Vec<u8> {
        if !self.weirdnet {
            return vec![];
        }
        let status = self.node().map(|n| n.status()).unwrap_or_default();
        let mut out = Vec::new();
        for (i, r) in weirdnet_device_link::KNOWN_RELAYS.iter().enumerate() {
            let peer = relay_peer(r);
            let on = status.reservations.iter().chain(status.listening.iter()).any(|a| a.contains(peer));
            // Two entries for one relay (TCP and QUIC): keep the first that is on.
            if (on || !strict) && !out.iter().any(|j: &u8| relay_peer(weirdnet_device_link::KNOWN_RELAYS[*j as usize]) == peer) {
                out.push(i as u8);
            }
        }
        out
    }

    fn roots(&self) -> Vec<PathBuf> {
        lock(&self.settings).get().roots.clone()
    }

    fn hello(&self) -> Hello {
        let (roots, confirm_shell) = {
            let mut s = lock(&self.settings);
            let s = s.get();
            (s.roots.clone(), s.confirm_shell)
        };
        Hello {
            version: VERSION,
            label: self.label.clone(),
            roots: roots
                .iter()
                .map(|p| RootInfo {
                    name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string()),
                    path: p.display().to_string(),
                })
                .collect(),
            confirm_shell,
            direct: self.direct(),
            relays: self.relays_now(true),
        }
    }

    fn root(&self, i: usize) -> Result<Root> {
        let roots = self.roots();
        let p = roots.get(i).ok_or_else(|| CoreError::new(ErrorKind::NotFound, "that project isn't shared any more"))?;
        Root::new(p)
    }

    /// The paired device making this call, or why not.
    fn device(&self, peer: &PeerId, req: &AppRequest) -> Result<Device> {
        let token = req
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .and_then(|(_, v)| v.strip_prefix("Bearer "))
            .ok_or_else(|| CoreError::new(ErrorKind::NotPaired, "not paired"))?;
        let hash = blake3::hash(token.as_bytes());
        let peer = peer.to_string();
        let mut devs = lock(&self.devices);
        let found = devs.get().iter_mut().find(|d| blake3::Hash::from_hex(&d.token_hash).is_ok_and(|h| h == hash) && d.peer == peer);
        let Some(d) = found else {
            return Err(CoreError::new(ErrorKind::NotPaired, "this phone isn't paired with this desktop (or was unpaired)"));
        };
        d.last_seen_ms = now_ms();
        let d = d.clone();
        let mut saved = lock(&self.last_saved);
        if now_ms() - *saved > 60_000 {
            *saved = now_ms();
            let all = devs.get().clone();
            if write_json(&self.dir.join("devices.json"), &all).is_ok() {
                devs.wrote();
            }
        }
        Ok(d)
    }

    fn pair(&self, peer: &PeerId, body: &[u8]) -> Result<PairReply> {
        let req: PairRequest = parse(body)?;
        {
            let mut pending = lock(&self.pending);
            let Some(p) = pending.as_mut() else {
                return Err(CoreError::new(ErrorKind::Pairing, "no pairing code is on show on the desktop"));
            };
            if p.expires_ms <= now_ms() {
                *pending = None;
                return Err(CoreError::new(ErrorKind::Pairing, "that pairing code expired; show a new one on the desktop"));
            }
            let given = req.secret.trim().to_ascii_lowercase();
            if !(same(given.as_bytes(), hex(&p.secret).as_bytes()) || same(given.as_bytes(), hex(&p.short).as_bytes())) {
                p.tries += 1;
                if p.tries >= MAX_TRIES {
                    *pending = None;
                    self.emit(LinkEvent::Refused("too many wrong pairing codes; the code was cancelled".into()));
                } else {
                    self.emit(LinkEvent::Refused(format!("a wrong pairing code from {peer}")));
                }
                return Err(CoreError::new(ErrorKind::Pairing, "that pairing code is wrong"));
            }
            *pending = None;
        }
        let token = hex(&random_bytes::<32>());
        let label = {
            let l: String = req.label.trim().chars().filter(|c| !c.is_control()).take(64).collect();
            if l.is_empty() { "Phone".to_string() } else { l }
        };
        let dev = Device {
            id: crate::core::random_id("d"),
            label,
            peer: peer.to_string(),
            token_hash: blake3::hash(token.as_bytes()).to_hex().to_string(),
            paired_ms: now_ms(),
            last_seen_ms: now_ms(),
        };
        {
            let mut devs = lock(&self.devices);
            let list = devs.get();
            // One record per phone key: pairing again replaces the old token.
            list.retain(|d| d.peer != dev.peer);
            list.push(dev.clone());
            let all = list.clone();
            write_json(&self.dir.join("devices.json"), &all)?;
            devs.wrote();
        }
        self.emit(LinkEvent::Paired(DeviceView::from(&dev)));
        Ok(PairReply { device_id: dev.id, token, desktop: self.hello() })
    }

    async fn handle(self: Arc<Self>, peer: PeerId, req: AppRequest) -> AppResponse {
        let method = req.method.to_ascii_uppercase();
        let path = req.path_with_query.split('?').next().unwrap_or("").to_string();
        if method == "POST" && path == "/v1/pair" {
            let body = match req.body.collect().await {
                Ok(b) => b,
                Err(e) => return error(&CoreError::new(ErrorKind::BadRequest, e.to_string())),
            };
            return match self.pair(&peer, &body) {
                Ok(r) => json(200, &r),
                Err(e) => error(&e),
            };
        }
        let dev = match self.device(&peer, &req) {
            Ok(d) => d,
            Err(e) => {
                self.emit(LinkEvent::Refused(format!("a call from an unpaired peer {peer}")));
                return error(&e);
            }
        };
        let body = match req.body.collect().await {
            Ok(b) => b,
            Err(e) => return error(&CoreError::new(ErrorKind::BadRequest, e.to_string())),
        };
        if method == "POST" && path == "/v1/shell" {
            return match self.shell(&dev, &body) {
                Ok(r) => r,
                Err(e) => error(&e),
            };
        }
        let s = self.clone();
        let r = tokio::task::spawn_blocking(move || s.call(&dev, &method, &path, &body)).await;
        match r {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => error(&e),
            Err(e) => error(&CoreError::new(ErrorKind::Other, e.to_string())),
        }
    }

    /// Every call but pairing and the shell (file work, off the async threads).
    fn call(&self, dev: &Device, method: &str, path: &str, body: &[u8]) -> Result<AppResponse> {
        let journal = |root: &Root| workspace::journal_for(&self.dir.join("journal"), root);
        Ok(match (method, path) {
            ("GET", "/v1/hello") | ("POST", "/v1/hello") => json(200, &self.hello()),
            ("POST", "/v1/list") => {
                let r: PathRequest = parse(body)?;
                json(200, &ListReply { entries: workspace::list(&self.root(r.root)?, &r.path)? })
            }
            ("POST", "/v1/read") => {
                let r: PathRequest = parse(body)?;
                json(200, &ReadReply { text: workspace::read(&self.root(r.root)?, &r.path)? })
            }
            ("POST", "/v1/gather") => {
                let r: GatherRequest = parse(body)?;
                json(200, &workspace::gather(&self.root(r.root)?, &r.targets)?)
            }
            ("POST", "/v1/apply") => {
                let r: ApplyRequest = parse(body)?;
                let root = self.root(r.root)?;
                let a = workspace::apply(&root, &journal(&root), &r.changes)?;
                self.emit(LinkEvent::Wrote { device: dev.label.clone(), files: a.files.clone() });
                json(200, &a)
            }
            ("POST", "/v1/undo") => {
                let r: UndoRequest = parse(body)?;
                let root = self.root(r.root)?;
                let a = workspace::undo(&root, &journal(&root), r.force)?;
                self.emit(LinkEvent::Wrote { device: dev.label.clone(), files: a.files.clone() });
                json(200, &a)
            }
            ("POST", "/v1/project") => {
                let r: PathRequest = parse(body)?;
                json(200, &workspace::project(&self.root(r.root)?, &r.path)?)
            }
            ("POST", "/v1/unpair") => {
                let mut devs = lock(&self.devices);
                let list = devs.get();
                list.retain(|d| d.id != dev.id);
                let all = list.clone();
                write_json(&self.dir.join("devices.json"), &all)?;
                devs.wrote();
                self.emit(LinkEvent::Unpaired(DeviceView::from(dev)));
                json(200, &serde_json::json!({}))
            }
            _ => return Err(CoreError::new(ErrorKind::NotFound, format!("no such call {method} {path}"))),
        })
    }

    /// Runs a command for a phone, streaming [`ShellFrame`]s as newline-delimited JSON. The call
    /// dropping (the phone pressed stop, or went away) kills the command and what it started.
    fn shell(self: &Arc<Self>, dev: &Device, body: &[u8]) -> Result<AppResponse> {
        let r: ShellRequest = parse(body)?;
        if r.command.trim().is_empty() {
            return Err(CoreError::new(ErrorKind::BadRequest, "no command"));
        }
        let root = self.root(r.root)?;
        let cwd = root.resolve(&r.cwd)?;
        let confirm_on = lock(&self.settings).get().confirm_shell;
        let ask = ShellAsk { device: dev.label.clone(), root: root.path().to_path_buf(), cwd: r.cwd.clone(), command: r.command.clone() };
        let confirm = self.confirm.clone();
        let timeout = r.timeout_secs.map(Duration::from_secs).unwrap_or(crate::shell::DEFAULT_TIMEOUT).min(Duration::from_secs(4 * 3600));
        let (tx, rx) = futures::channel::mpsc::unbounded::<std::result::Result<Vec<u8>, AppError>>();
        let me = self.clone();
        let device = dev.label.clone();
        std::thread::spawn(move || {
            let send = |f: &ShellFrame| {
                let mut line = serde_json::to_vec(f).unwrap_or_default();
                line.push(b'\n');
                tx.unbounded_send(Ok(line)).is_ok()
            };
            if confirm_on {
                let Some(confirm) = confirm else {
                    send(&ShellFrame::Denied { message: "the desktop asks before running commands, and nobody is there to ask".into() });
                    return;
                };
                send(&ShellFrame::Waiting);
                let answer = confirm(ask);
                // Keep the call alive while the desktop's user decides.
                let yes = futures::executor::block_on(async {
                    let mut answer = answer;
                    loop {
                        let tick = futures_timer(Duration::from_secs(2));
                        futures::pin_mut!(tick);
                        match futures::future::select(answer, tick).await {
                            futures::future::Either::Left((yes, _)) => break Some(yes),
                            futures::future::Either::Right((_, rest)) => {
                                if !send(&ShellFrame::Waiting) {
                                    break None;
                                }
                                answer = rest;
                            }
                        }
                    }
                });
                match yes {
                    Some(true) => {}
                    Some(false) => {
                        send(&ShellFrame::Denied { message: "the desktop said no".into() });
                        return;
                    }
                    None => return,
                }
            }
            me.emit(LinkEvent::Shell { device, command: r.command.clone() });
            let last = Mutex::new(std::time::Instant::now());
            let closed = || {
                if tx.is_closed() {
                    return true;
                }
                let mut l = lock(&last);
                // A frame every second while quiet: writing it is how a dropped call is noticed
                // (the network side only finds the stream reset when it writes), and it keeps
                // the call from going idle.
                if l.elapsed() > Duration::from_secs(1) {
                    *l = std::time::Instant::now();
                    return !send(&ShellFrame::Alive);
                }
                false
            };
            let ended = crate::shell::run_streaming(&r.command, &cwd, timeout, &closed, &mut |err, text| {
                *lock(&last) = std::time::Instant::now();
                send(&if err { ShellFrame::Err { text } } else { ShellFrame::Out { text } });
            });
            match ended {
                Ok(e) => send(&ShellFrame::Exit { code: e.code, timed_out: e.timed_out, stopped: e.stopped, elapsed_ms: e.elapsed_ms }),
                Err(e) => {
                    send(&ShellFrame::Err { text: format!("{}\n", e.message) })
                        && send(&ShellFrame::Exit { code: None, timed_out: false, stopped: false, elapsed_ms: 0 })
                }
            };
        });
        Ok(AppResponse::new(200).header("content-type", "application/x-ndjson").body(weirdnet_net::AppBody::stream(rx)))
    }
}

/// A timer that needs no runtime (the confirm wait runs on a plain thread).
fn futures_timer(d: Duration) -> impl std::future::Future<Output = ()> {
    let (tx, rx) = futures::channel::oneshot::channel::<()>();
    std::thread::spawn(move || {
        std::thread::sleep(d);
        let _ = tx.send(());
    });
    async move {
        let _ = rx.await;
    }
}
