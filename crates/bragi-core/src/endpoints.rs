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
use std::sync::Mutex;

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use serde::{Deserialize, Serialize};

use crate::core::{lock, random_id};
use crate::error::{CoreError, ErrorKind, Result};
use crate::openai;

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
}

fn storage(e: impl std::fmt::Display) -> CoreError {
    CoreError::new(ErrorKind::Storage, e.to_string())
}

impl Endpoints {
    pub fn open(dir: &Path) -> Self {
        let list = std::fs::read_to_string(dir.join(ENDPOINTS_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self { dir: dir.to_path_buf(), list: Mutex::new(list) }
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
        };
        let mut list = lock(&self.list);
        if list.len() >= MAX_ENDPOINTS {
            return Err(CoreError::new(ErrorKind::BadRequest, format!("at most {MAX_ENDPOINTS} endpoints")));
        }
        list.push(e.clone());
        self.save(&list)?;
        drop(list);
        let key = api_key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
        let has_key = key.is_some();
        if let Some(k) = key {
            self.set_key(&e.id, Some(&k))?;
        }
        Ok(view(&e, has_key))
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
        let has_key = self.read_secrets().unwrap_or_default().contains_key(id);
        Ok(view(&e, has_key))
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
        let api_key = self.read_secrets()?.remove(id);
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

    fn set_key(&self, id: &str, key: Option<&str>) -> Result<()> {
        let mut m = self.read_secrets().unwrap_or_default();
        match key {
            Some(k) => m.insert(id.to_string(), k.to_string()),
            None => m.remove(id),
        };
        if m.is_empty() {
            let _ = std::fs::remove_file(self.dir.join(SECRETS_FILE));
            return Ok(());
        }
        let mut nonce = [0u8; 12];
        getrandom::fill(&mut nonce).map_err(|e| storage(format!("random nonce: {e}")))?;
        let plain = serde_json::to_vec(&m).map_err(storage)?;
        let ct = self.cipher()?.encrypt(Nonce::from_slice(&nonce), plain.as_slice()).map_err(|_| storage("sealing the key store"))?;
        let mut out = nonce.to_vec();
        out.extend(ct);
        crate::identity::write_private_file(&self.dir.join(SECRETS_FILE), &out)?;
        Ok(())
    }
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
