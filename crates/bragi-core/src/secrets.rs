//! Where API keys can live besides the sealed file (LF-11): the operating system's keychain.
//!
//! [`SecretStore`] is a tiny get/set/delete by account name. [`os_keychain`] gives the platform's
//! store: Secret Service (libsecret) on Linux, Keychain on macOS and Credential Manager on Windows.
//! iOS and Android have no store here yet, so they keep the sealed file. A shell opts in with
//! `Core::use_os_keychain`; the core never touches a keychain on its own, so tests and demo
//! profiles stay off the real one. A value is never logged.

use std::sync::Arc;

use crate::error::{CoreError, ErrorKind, Result};

/// The keychain service name every Bragi secret is filed under.
pub const SERVICE: &str = "bragi";

/// A place to keep one secret per account name.
pub trait SecretStore: Send + Sync {
    /// Short name for the settings screen ("OS keychain").
    fn name(&self) -> &'static str;
    fn get(&self, account: &str) -> Result<Option<String>>;
    fn set(&self, account: &str, secret: &str) -> Result<()>;
    /// Removes it; a secret that isn't there is fine.
    fn delete(&self, account: &str) -> Result<()>;
}

fn store_err(e: impl std::fmt::Display) -> CoreError {
    CoreError::new(ErrorKind::Storage, format!("keychain: {e}"))
}

/// The platform keychain, or `None` when there is none to use (no Secret Service running, or a
/// platform without one here). Checks it by reading a name that is never set.
pub fn os_keychain() -> Option<Arc<dyn SecretStore>> {
    #[cfg(all(any(target_os = "linux", target_os = "macos", target_os = "windows"), not(target_os = "android")))]
    {
        let s: Arc<dyn SecretStore> = Arc::new(Os);
        s.get("probe").ok()?;
        Some(s)
    }
    #[cfg(not(all(any(target_os = "linux", target_os = "macos", target_os = "windows"), not(target_os = "android"))))]
    {
        None
    }
}

#[cfg(all(any(target_os = "linux", target_os = "macos", target_os = "windows"), not(target_os = "android")))]
struct Os;

#[cfg(all(any(target_os = "linux", target_os = "macos", target_os = "windows"), not(target_os = "android")))]
impl SecretStore for Os {
    fn name(&self) -> &'static str {
        "OS keychain"
    }

    fn get(&self, account: &str) -> Result<Option<String>> {
        match keyring::Entry::new(SERVICE, account).map_err(store_err)?.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_err(e)),
        }
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        keyring::Entry::new(SERVICE, account).map_err(store_err)?.set_password(secret).map_err(store_err)
    }

    fn delete(&self, account: &str) -> Result<()> {
        match keyring::Entry::new(SERVICE, account).map_err(store_err)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(store_err(e)),
        }
    }
}

/// An in-memory store, for tests of the code that uses one.
#[derive(Default)]
pub struct MemoryStore {
    map: std::sync::Mutex<std::collections::BTreeMap<String, String>>,
    /// When set, every call fails (a locked or vanished keychain).
    pub broken: std::sync::atomic::AtomicBool,
}

impl MemoryStore {
    fn check(&self) -> Result<()> {
        if self.broken.load(std::sync::atomic::Ordering::SeqCst) { Err(store_err("unavailable")) } else { Ok(()) }
    }

    pub fn len(&self) -> usize {
        crate::core::lock(&self.map).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl SecretStore for MemoryStore {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn get(&self, account: &str) -> Result<Option<String>> {
        self.check()?;
        Ok(crate::core::lock(&self.map).get(account).cloned())
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        self.check()?;
        crate::core::lock(&self.map).insert(account.into(), secret.into());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<()> {
        self.check()?;
        crate::core::lock(&self.map).remove(account);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a running keychain (Secret Service on Linux), so it only runs on request:
    /// `cargo test -p bragi-core -- --ignored os_keychain`. Run it in a private session bus with its
    /// own keyring daemon, never against the real login keyring.
    #[test]
    #[ignore = "needs a keychain"]
    fn os_keychain_round_trip() {
        let s = os_keychain().expect("a keychain is running");
        let acct = format!("test:{}", crate::core::random_id("k"));
        assert_eq!(s.get(&acct).unwrap(), None);
        s.set(&acct, "sk-test-value").unwrap();
        assert_eq!(s.get(&acct).unwrap().as_deref(), Some("sk-test-value"));
        s.set(&acct, "sk-other").unwrap();
        assert_eq!(s.get(&acct).unwrap().as_deref(), Some("sk-other"));
        s.delete(&acct).unwrap();
        s.delete(&acct).unwrap();
        assert_eq!(s.get(&acct).unwrap(), None);
    }

    #[test]
    fn memory_store_works() {
        let m = MemoryStore::default();
        m.set("a", "1").unwrap();
        assert_eq!(m.get("a").unwrap().as_deref(), Some("1"));
        m.delete("a").unwrap();
        assert!(m.is_empty());
    }
}
