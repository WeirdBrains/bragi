//! OpenAI-compatible endpoints (LF-11): the list the user keeps in Settings, and their API keys.
//!
//! The list (`endpoints.json`) holds names, addresses, a default model, the mode and the order;
//! the order is the order endpoints are tried as fallbacks. API keys are not in it: they live in
//! `secrets.enc`, sealed with ChaCha20-Poly1305 under a random key in `secrets.key` (owner-only
//! file). That is the encrypted-store fallback; an OS keychain would replace the key file. Neither
//! file is part of the sync records (endpoints are per device), keys are never logged and the
//! views the shells draw only say whether a key is set.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};

use crate::core::{lock, random_id};
use crate::error::{CoreError, ErrorKind, Result};
use crate::openai;
use crate::secrets::SecretStore;

pub const ENDPOINTS_FILE: &str = "endpoints.json";
pub const SECRETS_FILE: &str = "secrets.enc";
pub const SECRETS_KEY_FILE: &str = "secrets.key";
/// Most endpoints a user can keep.
pub const MAX_ENDPOINTS: usize = 16;

/// When an endpoint is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum EndpointMode {
    /// Only when the Weirdnet pool can't answer (no node, or it fails before any text).
    #[default]
    Fallback,
    /// Offered next to the pool: a session can be bound to it and its models are listed.
    Alongside,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub id: String,
    pub name: String,
    pub base_url: String,
    /// The model used when a session doesn't pick one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub mode: EndpointMode,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// The key is in the OS keychain (otherwise, if there is one, in the sealed file).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub in_keychain: bool,
}

fn yes() -> bool {
    true
}

/// What the shells draw: no key, only whether one is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointView {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model: Option<String>,
    pub mode: EndpointMode,
    pub enabled: bool,
    pub has_key: bool,
    /// Plain http to another machine: the key and the chat can be read on the way.
    pub insecure: bool,
}

/// What the user typed to add or replace an endpoint.
#[derive(Clone, Serialize, Deserialize)]
pub struct EndpointInput {
    pub name: String,
    pub base_url: String,
    pub model: Option<String>,
    pub mode: EndpointMode,
    pub enabled: bool,
}

/// What an edit does with the stored key.
#[derive(Clone, PartialEq, Eq)]
pub enum KeyChange {
    Keep,
    Set(String),
    Clear,
}

impl std::fmt::Debug for KeyChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            KeyChange::Keep => "Keep",
            KeyChange::Set(_) => "Set(…)",
            KeyChange::Clear => "Clear",
        })
    }
}

impl std::fmt::Debug for EndpointInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EndpointInput").field("name", &self.name).field("base_url", &self.base_url).finish_non_exhaustive()
    }
}

pub struct Endpoints {
    dir: PathBuf,
    list: Mutex<Vec<Endpoint>>,
    /// The OS keychain, once a shell has opted in; keys go there instead of the sealed file.
    keychain: Mutex<Option<Arc<dyn SecretStore>>>,
}

fn storage(e: impl std::fmt::Display) -> CoreError {
    CoreError::new(ErrorKind::Storage, e.to_string())
}

impl Endpoints {
    pub fn open(dir: &Path) -> Self {
        let list = std::fs::read_to_string(dir.join(ENDPOINTS_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self { dir: dir.to_path_buf(), list: Mutex::new(list), keychain: Mutex::new(None) }
    }

    fn save(&self, list: &[Endpoint]) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let bytes = serde_json::to_vec_pretty(list).map_err(storage)?;
        crate::identity::write_private_file(&self.dir.join(ENDPOINTS_FILE), &bytes)?;
        Ok(())
    }

    pub fn list(&self) -> Vec<Endpoint> {
        lock(&self.list).clone()
    }

    pub fn get(&self, id: &str) -> Option<Endpoint> {
        lock(&self.list).iter().find(|e| e.id == id).cloned()
    }

    pub fn views(&self) -> Vec<EndpointView> {
        let keys = self.read_secrets().unwrap_or_default();
        self.list().into_iter().map(|e| view(&e, keys.contains_key(&e.id))).collect()
    }

    pub fn add(&self, input: EndpointInput, api_key: Option<String>) -> Result<EndpointView> {
        let e = Endpoint {
            id: random_id("ep"),
            name: clean_name(&input.name)?,
            base_url: openai::normalize_url(&input.base_url)?,
            model: input.model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty()),
            mode: input.mode,
            enabled: input.enabled,
            in_keychain: false,
        };
        let mut list = lock(&self.list);
        if list.len() >= MAX_ENDPOINTS {
            return Err(CoreError::new(ErrorKind::BadRequest, format!("at most {MAX_ENDPOINTS} endpoints")));
        }
        list.push(e.clone());
        self.save(&list)?;
        drop(list);
        let key = api_key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
        if let Some(k) = key {
            self.set_key(&e.id, Some(&k))?;
        }
        let e = self.get(&e.id).unwrap_or(e);
        Ok(view(&e, self.has_key(&e)))
    }

    pub fn edit(&self, id: &str, input: EndpointInput, key: KeyChange) -> Result<EndpointView> {
        let name = clean_name(&input.name)?;
        let base_url = openai::normalize_url(&input.base_url)?;
        let mut list = lock(&self.list);
        let e = list.iter_mut().find(|e| e.id == id).ok_or_else(|| missing(id))?;
        e.name = name;
        e.base_url = base_url;
        e.model = input.model.map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
        e.mode = input.mode;
        e.enabled = input.enabled;
        let e = e.clone();
        self.save(&list)?;
        drop(list);
        match key {
            KeyChange::Keep => {}
            KeyChange::Set(k) if !k.trim().is_empty() => self.set_key(id, Some(k.trim()))?,
            KeyChange::Set(_) | KeyChange::Clear => self.set_key(id, None)?,
        }
        let e = self.get(id).unwrap_or(e);
        Ok(view(&e, self.has_key(&e)))
    }

    /// Removes the endpoint and its key. Sessions bound to it are the caller's to rebind.
    pub fn delete(&self, id: &str) -> Result<()> {
        let mut list = lock(&self.list);
        let n = list.len();
        list.retain(|e| e.id != id);
        if list.len() == n {
            return Err(missing(id));
        }
        self.save(&list)?;
        drop(list);
        self.set_key(id, None)
    }

    /// Moves an endpoint to `index` in the order (clamped).
    pub fn move_to(&self, id: &str, index: usize) -> Result<()> {
        let mut list = lock(&self.list);
        let from = list.iter().position(|e| e.id == id).ok_or_else(|| missing(id))?;
        let e = list.remove(from);
        let at = index.min(list.len());
        list.insert(at, e);
        self.save(&list)
    }

    /// Where to call, with the unsealed key (kept only for the call).
    pub fn target(&self, id: &str) -> Result<openai::Target> {
        let e = self.get(id).ok_or_else(|| missing(id))?;
        let api_key = self.read_key(&e)?;
        Ok(openai::Target { base_url: e.base_url, api_key })
    }

    /// Enabled fallback endpoints, in order.
    pub fn fallbacks(&self) -> Vec<Endpoint> {
        self.list().into_iter().filter(|e| e.enabled && e.mode == EndpointMode::Fallback).collect()
    }

    // ---- the sealed key store ----

    fn cipher(&self) -> Result<ChaCha20Poly1305> {
        let path = self.dir.join(SECRETS_KEY_FILE);
        let key = match std::fs::read(&path) {
            Ok(k) if k.len() == 32 => k,
            _ => {
                let mut k = [0u8; 32];
                getrandom::fill(&mut k).map_err(|e| storage(format!("random key: {e}")))?;
                std::fs::create_dir_all(&self.dir)?;
                crate::identity::write_private_file(&path, &k)?;
                k.to_vec()
            }
        };
        Ok(ChaCha20Poly1305::new(Key::from_slice(&key)))
    }

    fn read_secrets(&self) -> Result<BTreeMap<String, String>> {
        let Ok(bytes) = std::fs::read(self.dir.join(SECRETS_FILE)) else { return Ok(BTreeMap::new()) };
        if bytes.len() < 13 {
            return Err(storage("the key store is damaged"));
        }
        let (nonce, ct) = bytes.split_at(12);
        let plain = self.cipher()?.decrypt(Nonce::from_slice(nonce), ct).map_err(|_| storage("the key store can't be opened"))?;
        serde_json::from_slice(&plain).map_err(storage)
    }

    /// Whether a key is stored for it, without opening the keychain.
    pub fn has_key(&self, e: &Endpoint) -> bool {
        e.in_keychain || self.read_secrets().is_ok_and(|m| m.contains_key(&e.id))
    }

    /// Where a shell says keys should go from now on. Keys already in the sealed file move into the
    /// keychain (the file is removed once empty); one that can't move stays where it is. Returns
    /// how many moved.
    pub fn use_keychain(&self, store: Arc<dyn SecretStore>) -> usize {
        *lock(&self.keychain) = Some(store.clone());
        let Ok(mut sealed) = self.read_secrets() else { return 0 };
        let mut moved = 0;
        for id in sealed.keys().cloned().collect::<Vec<_>>() {
            if store.set(&account(&id), &sealed[&id]).is_ok() {
                sealed.remove(&id);
                self.flag_keychain(&id, true);
                moved += 1;
            }
        }
        if moved > 0 {
            let _ = self.write_secrets(&sealed);
        }
        moved
    }

    fn flag_keychain(&self, id: &str, on: bool) {
        let mut list = lock(&self.list);
        if let Some(e) = list.iter_mut().find(|e| e.id == id)
            && e.in_keychain != on
        {
            e.in_keychain = on;
            let _ = self.save(&list);
        }
    }

    fn read_key(&self, e: &Endpoint) -> Result<Option<String>> {
        if e.in_keychain {
            let store = lock(&self.keychain).clone().ok_or_else(|| storage("this key is in the OS keychain, which isn't available here"))?;
            return store.get(&account(&e.id));
        }
        Ok(self.read_secrets()?.remove(&e.id))
    }

    fn set_key(&self, id: &str, key: Option<&str>) -> Result<()> {
        let store = lock(&self.keychain).clone();
        if let (Some(store), Some(k)) = (&store, key) {
            if store.set(&account(id), k).is_ok() {
                self.flag_keychain(id, true);
                return self.drop_sealed(id);
            }
            tracing::warn!("the OS keychain refused a key; keeping it in the sealed file");
        }
        if let (Some(store), None) = (&store, key)
            && store.delete(&account(id)).is_err()
        {
            tracing::warn!("couldn't remove a key from the OS keychain");
        }
        self.flag_keychain(id, false);
        let mut m = self.read_secrets().unwrap_or_default();
        match key {
            Some(k) => m.insert(id.to_string(), k.to_string()),
            None => m.remove(id),
        };
        self.write_secrets(&m)
    }

    /// A secret kept for another feature (an MCP server's token) under `name`: the keychain when a
    /// shell opted in, else the sealed file. `None` removes it.
    pub(crate) fn put_secret(&self, name: &str, value: Option<&str>) -> Result<()> {
        let acct = format!("secret:{name}");
        let store = lock(&self.keychain).clone();
        if let Some(store) = &store {
            match value {
                Some(v) if store.set(&acct, v).is_ok() => return self.drop_sealed(&acct),
                Some(_) => tracing::warn!("the OS keychain refused a secret; keeping it in the sealed file"),
                None => {
                    let _ = store.delete(&acct);
                }
            }
        }
        let mut m = self.read_secrets().unwrap_or_default();
        match value {
            Some(v) => m.insert(acct, v.to_string()),
            None => m.remove(&acct),
        };
        self.write_secrets(&m)
    }

    /// Reads what [`put_secret`](Self::put_secret) stored.
    pub(crate) fn get_secret(&self, name: &str) -> Result<Option<String>> {
        let acct = format!("secret:{name}");
        if let Some(store) = lock(&self.keychain).clone()
            && let Some(v) = store.get(&acct)?
        {
            return Ok(Some(v));
        }
        Ok(self.read_secrets()?.remove(&acct))
    }

    fn drop_sealed(&self, id: &str) -> Result<()> {
        let mut m = self.read_secrets().unwrap_or_default();
        if m.remove(id).is_some() {
            self.write_secrets(&m)?;
        }
        Ok(())
    }

    fn write_secrets(&self, m: &BTreeMap<String, String>) -> Result<()> {
        if m.is_empty() {
            let _ = std::fs::remove_file(self.dir.join(SECRETS_FILE));
            let _ = std::fs::remove_file(self.dir.join(SECRETS_KEY_FILE));
            return Ok(());
        }
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).map_err(|e| storage(format!("random nonce: {e}")))?;
        let plain = serde_json::to_vec(m).map_err(storage)?;
        let ct = self.cipher()?.encrypt(Nonce::from_slice(&nonce), plain.as_slice()).map_err(|_| storage("sealing the key store"))?;
        let mut out = nonce.to_vec();
        out.extend(ct);
        crate::identity::write_private_file(&self.dir.join(SECRETS_FILE), &out)?;
        Ok(())
    }
}

/// The keychain account for an endpoint's key.
fn account(id: &str) -> String {
    format!("endpoint:{id}")
}

fn view(e: &Endpoint, has_key: bool) -> EndpointView {
    EndpointView {
        id: e.id.clone(),
        name: e.name.clone(),
        base_url: e.base_url.clone(),
        model: e.model.clone(),
        mode: e.mode,
        enabled: e.enabled,
        has_key,
        insecure: openai::is_insecure(&e.base_url),
    }
}

fn clean_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 60 {
        return Err(CoreError::new(ErrorKind::BadRequest, "give the endpoint a name (up to 60 characters)"));
    }
    Ok(n.to_string())
}

fn missing(id: &str) -> CoreError {
    CoreError::new(ErrorKind::NotFound, format!("no endpoint `{id}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str) -> EndpointInput {
        EndpointInput { name: name.into(), base_url: "https://api.example.com/v1/".into(), model: None, mode: EndpointMode::Fallback, enabled: true }
    }

    #[test]
    fn keys_are_sealed_and_never_in_the_list() {
        let d = tempfile::tempdir().unwrap();
        let s = Endpoints::open(d.path());
        let v = s.add(input("A"), Some("sk-very-secret".into())).unwrap();
        assert!(v.has_key);
        for f in [ENDPOINTS_FILE, SECRETS_FILE, SECRETS_KEY_FILE] {
            let bytes = std::fs::read(d.path().join(f)).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("sk-very-secret"), "{f}");
        }
        assert_eq!(s.target(&v.id).unwrap().api_key.as_deref(), Some("sk-very-secret"));
        // A new store over the same folder reads it back.
        assert_eq!(Endpoints::open(d.path()).target(&v.id).unwrap().api_key.as_deref(), Some("sk-very-secret"));
        assert!(!format!("{:?}", KeyChange::Set("sk-very-secret".into())).contains("secret"));
    }

    #[test]
    fn edit_keep_set_clear_delete() {
        let d = tempfile::tempdir().unwrap();
        let s = Endpoints::open(d.path());
        let v = s.add(input("A"), Some("k1".into())).unwrap();
        assert!(s.edit(&v.id, input("B"), KeyChange::Keep).unwrap().has_key);
        assert_eq!(s.target(&v.id).unwrap().api_key.as_deref(), Some("k1"));
        s.edit(&v.id, input("B"), KeyChange::Set("k2".into())).unwrap();
        assert_eq!(s.target(&v.id).unwrap().api_key.as_deref(), Some("k2"));
        assert!(!s.edit(&v.id, input("B"), KeyChange::Clear).unwrap().has_key);
        assert_eq!(s.target(&v.id).unwrap().api_key, None);
        s.edit(&v.id, input("B"), KeyChange::Set("k3".into())).unwrap();
        s.delete(&v.id).unwrap();
        assert!(s.list().is_empty());
        assert!(!d.path().join(SECRETS_FILE).exists(), "the last key's file goes with it");
        assert!(s.delete(&v.id).is_err());
    }

    #[test]
    fn keychain_holds_keys_and_the_sealed_file_is_the_fallback() {
        use crate::secrets::MemoryStore;
        use std::sync::atomic::Ordering;
        let d = tempfile::tempdir().unwrap();
        let s = Endpoints::open(d.path());
        // A key set before the keychain was chosen moves into it.
        let old = s.add(input("Old"), Some("sk-old-secret".into())).unwrap();
        assert!(d.path().join(SECRETS_FILE).exists());
        let kc = Arc::new(MemoryStore::default());
        assert_eq!(s.use_keychain(kc.clone()), 1);
        assert!(!d.path().join(SECRETS_FILE).exists() && !d.path().join(SECRETS_KEY_FILE).exists(), "nothing sealed is left");
        assert_eq!(s.target(&old.id).unwrap().api_key.as_deref(), Some("sk-old-secret"));
        // New keys go to the keychain and never touch the disk.
        let v = s.add(input("New"), Some("sk-new-secret".into())).unwrap();
        assert!(v.has_key && kc.len() == 2 && !d.path().join(SECRETS_FILE).exists());
        let list = std::fs::read_to_string(d.path().join(ENDPOINTS_FILE)).unwrap();
        assert!(!list.contains("secret") && list.contains("in_keychain"));
        // Clear and delete remove them from the keychain.
        assert!(!s.edit(&v.id, input("New"), KeyChange::Clear).unwrap().has_key);
        s.delete(&old.id).unwrap();
        assert!(kc.is_empty());
        // A keychain that refuses writes: the key is sealed in the file instead and still works.
        kc.broken.store(true, Ordering::SeqCst);
        let f = s.add(input("Fallback"), Some("sk-fb-secret".into())).unwrap();
        assert!(f.has_key && d.path().join(SECRETS_FILE).exists());
        assert_eq!(s.target(&f.id).unwrap().api_key.as_deref(), Some("sk-fb-secret"));
        // A key in the keychain with no keychain available is an error, not a silent "no key".
        kc.broken.store(false, Ordering::SeqCst);
        let k = s.add(input("K"), Some("sk-k-secret".into())).unwrap();
        let s2 = Endpoints::open(d.path());
        assert!(s2.target(&k.id).is_err());
        s2.use_keychain(kc);
        assert_eq!(s2.target(&k.id).unwrap().api_key.as_deref(), Some("sk-k-secret"));
    }

    #[test]
    fn order_and_validation() {
        let d = tempfile::tempdir().unwrap();
        let s = Endpoints::open(d.path());
        let a = s.add(input("A"), None).unwrap().id;
        let b = s.add(input("B"), None).unwrap().id;
        let c = s.add(input("C"), None).unwrap().id;
        s.move_to(&c, 0).unwrap();
        s.move_to(&a, 99).unwrap();
        let order: Vec<String> = Endpoints::open(d.path()).list().into_iter().map(|e| e.id).collect();
        assert_eq!(order, vec![c, b, a]);
        assert!(s.add(input("  "), None).is_err());
        let mut bad = input("X");
        bad.base_url = "nope".into();
        assert!(s.add(bad, None).is_err());
        assert_eq!(s.list()[0].base_url, "https://api.example.com/v1");
    }
}
