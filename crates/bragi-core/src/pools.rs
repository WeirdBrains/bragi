//! Saved Ymir pools (identities). One device can be linked to more than one cloud: each pool
//! keeps its own identity, node addresses and chat history. The shells list, add, switch and
//! remove them; the choice of which is active is remembered in `pools.json`.
//!
//! A profile that was linked before this existed keeps its files at the profile root (`path` is
//! `"."`). Later pools live under `pools/<id>/`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::core::{Core, PEERS_FILE, lock, random_id};
use crate::error::{CoreError, ErrorKind, Result};
use crate::identity::Identity;
use crate::sessions::SESSIONS_FILE;

const FILE: &str = "pools.json";
const DEFAULT_NAME: &str = "Ymir pool";

/// A saved pool, as Settings and the command line list it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolView {
    pub id: String,
    pub name: String,
    /// The user id this device is linked as, once pairing has finished.
    pub user: String,
    /// This device's label on that pool.
    pub device: String,
    pub active: bool,
    /// The identity is on disk (the device can go online as this pool).
    pub linked: bool,
}

/// The active pool, as [`crate::AppState`] carries it for the shells.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PoolRef {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Rec {
    id: String,
    name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    user: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    device: String,
    /// Relative to the profile directory. `"."` means the files stay at the profile root.
    #[serde(default = "dot")]
    path: String,
}

fn dot() -> String {
    ".".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct File {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active: Option<String>,
    #[serde(default)]
    pools: Vec<Rec>,
}

/// What a new-pool pairing is waiting to become, once the codes match.
#[derive(Clone)]
pub(crate) struct PendingPool {
    rec: Rec,
}

pub(crate) struct Pools {
    root: PathBuf,
    path: PathBuf,
    data: Mutex<File>,
    /// A pool being linked for the first time (not in the file until it succeeds).
    pending: Mutex<Option<PendingPool>>,
}

fn storage(e: impl std::fmt::Display) -> CoreError {
    CoreError::new(ErrorKind::Storage, e.to_string())
}

fn not_found() -> CoreError {
    CoreError::new(ErrorKind::NotFound, "no such pool")
}

fn legacy_at(root: &Path) -> bool {
    root.join("identity").is_dir() || root.join("history").exists() || root.join(PEERS_FILE).exists() || root.join(SESSIONS_FILE).exists()
}

impl Pools {
    pub fn open(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root).map_err(storage)?;
        let path = root.join(FILE);
        let mut data = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<File>(&b).ok()).unwrap_or_default();
        if data.pools.is_empty() && legacy_at(root) {
            let rec = Rec { id: random_id("p"), name: DEFAULT_NAME.into(), user: String::new(), device: String::new(), path: ".".into() };
            data.active = Some(rec.id.clone());
            data.pools.push(rec);
            let bytes = serde_json::to_vec_pretty(&data).map_err(storage)?;
            crate::identity::write_private_file(&path, &bytes).map_err(storage)?;
        }
        if data.active.is_none() && !data.pools.is_empty() {
            data.active = Some(data.pools[0].id.clone());
        }
        Ok(Self { root: root.to_path_buf(), path, data: Mutex::new(data), pending: Mutex::new(None) })
    }

    fn save(&self, data: &File) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(data).map_err(storage)?;
        crate::identity::write_private_file(&self.path, &bytes).map_err(storage)?;
        Ok(())
    }

    fn abs(&self, rec: &Rec) -> PathBuf {
        if rec.path == "." || rec.path.is_empty() { self.root.clone() } else { self.root.join(&rec.path) }
    }

    /// The directory of the active pool, or the profile root when none is saved yet.
    pub fn active_dir(&self) -> PathBuf {
        let data = lock(&self.data);
        data.pools.iter().find(|r| data.active.as_deref() == Some(r.id.as_str())).map(|r| self.abs(r)).unwrap_or_else(|| self.root.clone())
    }

    pub fn active_id(&self) -> Option<String> {
        lock(&self.data).active.clone()
    }

    fn view(&self, rec: &Rec, active: bool) -> PoolView {
        PoolView {
            id: rec.id.clone(),
            name: rec.name.clone(),
            user: rec.user.clone(),
            device: rec.device.clone(),
            active,
            linked: Identity::exists(&self.abs(rec).join("identity")),
        }
    }

    pub fn list(&self) -> Vec<PoolView> {
        let data = lock(&self.data);
        data.pools.iter().map(|r| self.view(r, data.active.as_deref() == Some(r.id.as_str()))).collect()
    }

    pub fn get(&self, id: &str) -> Option<PoolView> {
        self.list().into_iter().find(|p| p.id == id)
    }

    pub fn current(&self) -> Option<PoolView> {
        let data = lock(&self.data);
        let id = data.active.as_deref()?;
        data.pools.iter().find(|r| r.id == id).map(|r| self.view(r, true))
    }

    pub fn current_ref(&self) -> Option<PoolRef> {
        self.current().map(|p| PoolRef { id: p.id, name: p.name })
    }

    /// Finds a pool by id, or by name when the name is unique.
    pub fn resolve(&self, id_or_name: &str) -> Result<String> {
        let q = id_or_name.trim();
        if q.is_empty() {
            return Err(CoreError::new(ErrorKind::BadRequest, "name a pool"));
        }
        let data = lock(&self.data);
        if let Some(r) = data.pools.iter().find(|r| r.id == q) {
            return Ok(r.id.clone());
        }
        let hits: Vec<_> = data.pools.iter().filter(|r| r.name.eq_ignore_ascii_case(q)).collect();
        match hits.as_slice() {
            [r] => Ok(r.id.clone()),
            [] => Err(not_found()),
            _ => Err(CoreError::new(ErrorKind::BadRequest, "that name matches more than one pool; use the id")),
        }
    }

    fn unique_name(&self, want: &str) -> String {
        let data = lock(&self.data);
        let want = want.trim();
        let base = if want.is_empty() { DEFAULT_NAME } else { want };
        if !data.pools.iter().any(|r| r.name == base) {
            return base.to_string();
        }
        let mut n = 2;
        loop {
            let name = format!("{base} {n}");
            if !data.pools.iter().any(|r| r.name == name) {
                return name;
            }
            n += 1;
        }
    }

    /// A new, empty pool directory (not yet in the file) for a pairing in progress.
    pub fn begin_pending(&self) -> Result<PendingPool> {
        if lock(&self.pending).is_some() {
            return Err(CoreError::new(ErrorKind::Pairing, "a new pool is already being linked"));
        }
        let id = random_id("p");
        let rec =
            Rec { id: id.clone(), name: self.unique_name(DEFAULT_NAME), user: String::new(), device: String::new(), path: format!("pools/{id}") };
        let dir = self.abs(&rec);
        std::fs::create_dir_all(&dir).map_err(storage)?;
        let pending = PendingPool { rec };
        *lock(&self.pending) = Some(pending.clone());
        Ok(pending)
    }

    pub fn pending_dir(&self) -> Option<PathBuf> {
        lock(&self.pending).as_ref().map(|p| self.abs(&p.rec))
    }

    pub fn pending_id(&self) -> Option<String> {
        lock(&self.pending).as_ref().map(|p| p.rec.id.clone())
    }

    /// Drops the pending new pool only if it is still `id` (a later add is left alone).
    pub fn cancel_pending_if(&self, id: &str) {
        let mut g = lock(&self.pending);
        if g.as_ref().is_some_and(|p| p.rec.id == id)
            && let Some(p) = g.take()
        {
            drop(g);
            let dir = self.abs(&p.rec);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    pub fn take_pending_if(&self, id: &str) -> Option<PendingPool> {
        let mut g = lock(&self.pending);
        if g.as_ref().is_some_and(|p| p.rec.id == id) { g.take() } else { None }
    }

    pub fn take_pending(&self) -> Option<PendingPool> {
        lock(&self.pending).take()
    }

    /// Drops a cancelled new-pool pairing and its empty directory.
    pub fn cancel_pending(&self) {
        if let Some(p) = self.take_pending() {
            let dir = self.abs(&p.rec);
            // Only remove what we created; leave anything the user might have placed elsewhere.
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    /// Records a finished pending pool and makes it active.
    pub fn commit_pending(&self, pending: PendingPool, user: String, device: String) -> Result<Rec> {
        let mut rec = pending.rec;
        rec.user = user;
        rec.device = device;
        let mut data = lock(&self.data);
        if data.pools.iter().any(|r| r.id == rec.id) {
            return Err(CoreError::new(ErrorKind::Other, "that pool is already saved"));
        }
        data.active = Some(rec.id.clone());
        data.pools.push(rec.clone());
        self.save(&data)?;
        Ok(rec)
    }

    /// Makes sure an unpaired profile has a pool record at the root to receive the first link.
    pub fn ensure_first(&self) -> Result<Rec> {
        let mut data = lock(&self.data);
        if let Some(id) = data.active.as_deref()
            && let Some(r) = data.pools.iter().find(|r| r.id == id)
        {
            return Ok(r.clone());
        }
        if let Some(r) = data.pools.first().cloned() {
            data.active = Some(r.id.clone());
            self.save(&data)?;
            return Ok(r);
        }
        let rec = Rec {
            id: random_id("p"),
            name: self.unique_name_locked(&data, DEFAULT_NAME),
            user: String::new(),
            device: String::new(),
            path: ".".into(),
        };
        data.active = Some(rec.id.clone());
        data.pools.push(rec.clone());
        self.save(&data)?;
        Ok(rec)
    }

    fn unique_name_locked(&self, data: &File, want: &str) -> String {
        let base = if want.trim().is_empty() { DEFAULT_NAME } else { want.trim() };
        if !data.pools.iter().any(|r| r.name == base) {
            return base.to_string();
        }
        let mut n = 2;
        loop {
            let name = format!("{base} {n}");
            if !data.pools.iter().any(|r| r.name == name) {
                return name;
            }
            n += 1;
        }
    }

    pub fn set_active(&self, id: &str) -> Result<Rec> {
        let mut data = lock(&self.data);
        let rec = data.pools.iter().find(|r| r.id == id).cloned().ok_or_else(not_found)?;
        data.active = Some(rec.id.clone());
        self.save(&data)?;
        Ok(rec)
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<Rec> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CoreError::new(ErrorKind::BadRequest, "give the pool a name"));
        }
        let mut data = lock(&self.data);
        if data.pools.iter().any(|r| r.id != id && r.name == name) {
            return Err(CoreError::new(ErrorKind::BadRequest, "another pool already has that name"));
        }
        let rec = data.pools.iter_mut().find(|r| r.id == id).ok_or_else(not_found)?;
        rec.name = name.into();
        let rec = rec.clone();
        self.save(&data)?;
        Ok(rec)
    }

    pub fn set_meta(&self, id: &str, user: String, device: String) -> Result<()> {
        let mut data = lock(&self.data);
        let Some(rec) = data.pools.iter_mut().find(|r| r.id == id) else { return Ok(()) };
        rec.user = user;
        rec.device = device;
        self.save(&data)
    }

    /// Removes the record. The caller deletes the files (and switches away first if it was active).
    pub fn forget(&self, id: &str) -> Result<Rec> {
        let mut data = lock(&self.data);
        let at = data.pools.iter().position(|r| r.id == id).ok_or_else(not_found)?;
        let rec = data.pools.remove(at);
        if data.active.as_deref() == Some(id) {
            data.active = data.pools.first().map(|r| r.id.clone());
        }
        self.save(&data)?;
        Ok(rec)
    }

    /// Deletes a pool's identity, history, peers and sessions. When the files live at the
    /// profile root, only those entries are removed (themes, endpoints and other pools stay).
    pub fn wipe(root: &Path, rec: &Rec) -> Result<()> {
        let dir = if rec.path == "." || rec.path.is_empty() { root.to_path_buf() } else { root.join(&rec.path) };
        if rec.path == "." || rec.path.is_empty() {
            for name in ["identity", "history", "history.imported", PEERS_FILE, SESSIONS_FILE] {
                let p = dir.join(name);
                if p.is_dir() {
                    std::fs::remove_dir_all(&p).map_err(storage)?;
                } else if p.exists() {
                    std::fs::remove_file(&p).map_err(storage)?;
                }
            }
        } else if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(storage)?;
        }
        Ok(())
    }
}

impl Core {
    /// The saved Ymir pools, with the active one marked.
    pub fn pools(&self) -> Vec<PoolView> {
        self.inner.pools.list()
    }

    pub fn active_pool(&self) -> Option<PoolView> {
        self.inner.pools.current()
    }

    pub(crate) fn pool_dir(&self) -> PathBuf {
        self.inner.pools.active_dir()
    }

    pub(crate) fn publish_pool(&self) {
        let pool = self.inner.pools.current_ref();
        self.inner.state.send_modify(|s| s.pool = pool);
    }

    /// Finds a pool by id or unique name (for the command line).
    pub fn resolve_pool(&self, id_or_name: &str) -> Result<String> {
        self.inner.pools.resolve(id_or_name)
    }

    pub fn rename_pool(&self, id: &str, name: &str) -> Result<PoolView> {
        let rec = self.inner.pools.rename(id, name)?;
        self.publish_pool();
        self.inner.pools.get(&rec.id).ok_or_else(not_found)
    }

    /// Shows a link code that a device on another Ymir pool can open (`ymirctl pool add CODE`).
    /// Works while this device is already linked: the current pool stays active until the new
    /// one finishes pairing, then it becomes active.
    pub async fn add_pool(&self) -> Result<String> {
        if self.inner.pools.pending_dir().is_some() || lock(&self.inner.offer).is_some() {
            return Err(CoreError::new(ErrorKind::Pairing, "a link is already in progress"));
        }
        let _pending = self.inner.pools.begin_pending()?;
        match self.offer_into().await {
            Ok(link) => Ok(link),
            Err(e) => {
                self.inner.pools.cancel_pending();
                Err(e)
            }
        }
    }

    /// Switches which saved pool is active: this device goes online as that identity, and that
    /// pool's history and node addresses come into use. The choice is remembered.
    pub async fn switch_pool(&self, id: &str) -> Result<PoolView> {
        let id = self.inner.pools.resolve(id)?;
        if self.inner.pools.active_id().as_deref() == Some(id.as_str()) {
            return self.inner.pools.get(&id).ok_or_else(not_found);
        }
        if self.inner.pools.get(&id).is_none() {
            return Err(not_found());
        }
        self.activate(&id).await?;
        self.inner.pools.get(&id).ok_or_else(not_found)
    }

    /// Forgets a saved pool and deletes its identity and history on this device. If it was
    /// active, another saved pool becomes active, or this device goes unpaired.
    pub async fn remove_pool(&self, id: &str) -> Result<()> {
        let id = self.inner.pools.resolve(id)?;
        let was_active = self.inner.pools.active_id().as_deref() == Some(id.as_str());
        let rec = self.inner.pools.forget(&id)?;
        if was_active {
            self.go_offline().await;
            if let Some(next) = self.inner.pools.active_id() {
                // forget() already pointed active at another pool, if any.
                let _ = next;
                self.reload_active().await?;
            } else {
                self.reload_unpaired();
            }
        }
        Pools::wipe(&self.inner.cfg.dir, &rec)?;
        self.publish_pool();
        Ok(())
    }

    pub(crate) async fn go_offline(&self) {
        if let Some(h) = lock(&self.inner.offer).take() {
            h.cancel();
        }
        *lock(&self.inner.pairing) = None;
        {
            let mut reqs = lock(&self.inner.requests);
            for token in reqs.values() {
                token.cancel();
            }
            reqs.clear();
        }
        lock(&self.inner.cards).clear();
        *lock(&self.inner.net) = None;
        self.inner.history.detach();
        self.inner.state.send_modify(|s| {
            s.pair = crate::core::PairState::Unpaired;
            s.catalog = Default::default();
            s.online = false;
            s.node_search = Default::default();
        });
    }

    pub(crate) async fn activate(&self, id: &str) -> Result<()> {
        self.go_offline().await;
        self.inner.pools.set_active(id)?;
        self.reload_active().await
    }

    pub(crate) async fn reload_active(&self) -> Result<()> {
        let dir = self.inner.pools.active_dir();
        self.inner.history.switch_root(&dir);
        self.inner.sessions.switch_root(&dir);
        self.reload_peers(&dir);
        self.publish_pool();
        if Identity::exists(&dir.join("identity"))
            && let Err(e) = self.go_online().await
        {
            self.set_pair_failed(e.message.clone());
            return Err(e);
        }
        Ok(())
    }

    fn reload_unpaired(&self) {
        let dir = self.inner.pools.active_dir();
        self.inner.history.switch_root(&dir);
        self.inner.sessions.switch_root(&dir);
        self.reload_peers(&dir);
        self.publish_pool();
    }

    fn reload_peers(&self, dir: &Path) {
        let mut known = self.inner.cfg.peers.clone();
        crate::core::merge_peers(&mut known, crate::core::load_peers(dir));
        *lock(&self.inner.peers) = known;
    }

    fn set_pair_failed(&self, message: String) {
        self.inner.state.send_modify(|s| s.pair = crate::core::PairState::Failed { message });
    }

    /// The directory a pairing in progress should write the identity into.
    pub(crate) fn offer_pool_dir(&self) -> PathBuf {
        self.inner.pools.pending_dir().unwrap_or_else(|| self.pool_dir())
    }
}

// Rec is used by wipe/commit; keep it crate-visible through Pools methods only.
