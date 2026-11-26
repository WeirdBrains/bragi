//! Push wake-ups (B4) on `weirdnet-push`: a phone that is asleep can be woken to sync.
//!
//! Each device keeps a push key and a wake handle (`wake.key`, private) and shares a
//! [`DeviceWake`] card with the user's own linked devices only, as a sealed `weirdnet-sync`
//! record of type [`WAKE_TYPE`] (no cert-only server is granted it). A wake is 96 sealed bytes
//! carrying only a reason, a random value and a time; [`Wakes::open_wake`] refuses wakes for another
//! handle, wakes that don't open with this device's key, and repeats or stale ones
//! (`ReplayGuard`, 10 minutes). A valid wake runs a sync pass.
//!
//! [`WakeSettings::push`] (on by default) is the per-device switch: off withdraws the card,
//! forgets the key and handle, and a later on makes new ones, so wakes sealed for the old
//! handle no longer open. [`WakeSettings::relay_wake`] (off by default, opt-in, see
//! [`RELAY_WAKE_COST`]) keeps a `/weirdnet/wake/1` stream open to the home relay so wakes
//! arrive without Google or Apple services; on Android the app runs a foreground service while
//! it is on. Delivery through APNs and FCM needs the push service and platform tokens, which
//! these builds do not have yet.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use weirdnet_push::{PushKey, Reason, ReplayGuard, Wake, WakeCard, WakeHandle};

/// The sync type of wake cards: sealed, the user's full devices only.
pub const WAKE_TYPE: &str = "bragi.wake";
/// The push service named in cards (APNs/FCM delivery; not used by the relay wake path).
pub const DEFAULT_SERVICE: &str = "https://push.weirdbrains.com";
/// What the settings say before the relay wake path is turned on.
pub const RELAY_WAKE_COST: &str = "Wake through your relay keeps a connection to your home relay open while the app is in the background, so your other devices can wake this one without Google or Apple push. On Android it runs as a foreground service with a notification, and it uses more battery than push. Off: this device is reachable only while Bragi is open (or through push, where available).";
/// How long a wake stays valid, and how many recent ones are remembered against repeats.
const REPLAY_WINDOW: Duration = Duration::from_secs(600);
const REPLAY_MAX: usize = 256;
const SETTINGS_FILE: &str = "wake.json";
const KEY_FILE: &str = "wake.key";

/// The per-device wake settings (`<data dir>/wake.json`; not synced).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WakeSettings {
    /// This device can be woken at all (shares its card). On by default.
    #[serde(default = "yes")]
    pub push: bool,
    /// Keep a wake connection to the home relay (opt-in, off by default; see [`RELAY_WAKE_COST`]).
    #[serde(default)]
    pub relay_wake: bool,
}

fn yes() -> bool {
    true
}

impl Default for WakeSettings {
    fn default() -> Self {
        Self { push: true, relay_wake: false }
    }
}

impl WakeSettings {
    /// The relay wake path runs: it is on, and so is push.
    pub fn relay_active(&self) -> bool {
        self.push && self.relay_wake
    }
}

/// One device's wake card, as its sync record holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceWake {
    /// `None`: the device turned push off (the card is withdrawn).
    pub card: Option<WakeCard>,
    /// The relay it keeps a wake connection to (`/dns4/…/p2p/RELAY`), while relay wake is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay: Option<String>,
}

/// What the shells show about wake-ups.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WakeStatus {
    pub settings: WakeSettingsView,
    /// The relay wake connection is up.
    pub relay_connected: bool,
    /// When the last valid wake arrived (ms since the epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_wake_ms: Option<i64>,
    /// Wakes refused (another handle, a bad seal, a repeat or a stale one).
    pub refused: u64,
}

/// [`WakeSettings`] for [`WakeStatus`] (serde default differs: a state built empty is off).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WakeSettingsView {
    pub push: bool,
    pub relay_wake: bool,
}

impl From<WakeSettings> for WakeSettingsView {
    fn from(s: WakeSettings) -> Self {
        Self { push: s.push, relay_wake: s.relay_wake }
    }
}

#[derive(Serialize, Deserialize)]
struct StoredKey {
    key: String,
    handle: WakeHandle,
}

struct Keys {
    key: PushKey,
    handle: WakeHandle,
}

/// This device's wake key, handle, settings and replay guard.
pub struct Wakes {
    dir: PathBuf,
    settings: Mutex<WakeSettings>,
    keys: Mutex<Option<Keys>>,
    guard: Mutex<ReplayGuard>,
    refused: Mutex<u64>,
    recent: Mutex<VecDeque<i64>>,
}

impl Wakes {
    /// Opens the wake state in `dir` (the device's data dir). A missing or unreadable
    /// settings file reads as the defaults.
    pub fn open(dir: &Path) -> Result<Self> {
        let settings = std::fs::read(dir.join(SETTINGS_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        let keys = match std::fs::read(dir.join(KEY_FILE)) {
            Ok(b) => {
                let s: StoredKey = serde_json::from_slice(&b).context("reading wake.key")?;
                Some(Keys { key: PushKey::from_bytes(from_hex32(&s.key)?), handle: s.handle })
            }
            Err(_) => None,
        };
        Ok(Self {
            dir: dir.to_path_buf(),
            settings: Mutex::new(settings),
            keys: Mutex::new(keys),
            guard: Mutex::new(ReplayGuard::new(REPLAY_WINDOW, REPLAY_MAX)),
            refused: Mutex::new(0),
            recent: Mutex::new(VecDeque::new()),
        })
    }

    pub fn settings(&self) -> WakeSettings {
        *lock(&self.settings)
    }

    /// Saves new settings. Turning push off forgets the key and handle (`wake.key` is
    /// removed), so wakes sealed for them stop opening at once.
    pub fn set(&self, s: WakeSettings) -> Result<()> {
        std::fs::create_dir_all(&self.dir).with_context(|| format!("creating {}", self.dir.display()))?;
        std::fs::write(self.dir.join(SETTINGS_FILE), serde_json::to_vec_pretty(&s)?).context("writing wake.json")?;
        *lock(&self.settings) = s;
        if !s.push {
            *lock(&self.keys) = None;
            match std::fs::remove_file(self.dir.join(KEY_FILE)) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(anyhow!("removing wake.key: {e}")),
            }
        }
        Ok(())
    }

    /// This device's handle, made (with its key) the first time; `None` while push is off.
    pub fn handle(&self) -> Result<Option<WakeHandle>> {
        if !self.settings().push {
            return Ok(None);
        }
        let mut k = lock(&self.keys);
        if k.is_none() {
            let key = PushKey::generate();
            let handle = WakeHandle::generate();
            let stored = StoredKey { key: to_hex(&key.to_bytes()[..]), handle: handle.clone() };
            std::fs::create_dir_all(&self.dir).with_context(|| format!("creating {}", self.dir.display()))?;
            crate::identity::write_private_file(&self.dir.join(KEY_FILE), &zeroize::Zeroizing::new(serde_json::to_vec(&stored)?))
                .context("writing wake.key")?;
            *k = Some(Keys { key, handle });
        }
        Ok(k.as_ref().map(|k| k.handle.clone()))
    }

    /// The card this device shares now: withdrawn (`card: None`) while push is off.
    pub fn card(&self, relay: Option<&str>) -> Result<DeviceWake> {
        let Some(handle) = self.handle()? else { return Ok(DeviceWake { card: None, relay: None }) };
        let key = lock(&self.keys).as_ref().map(|k| to_hex(&k.key.public())).unwrap_or_default();
        let relay = if self.settings().relay_active() { relay.map(str::to_string) } else { None };
        Ok(DeviceWake { card: Some(WakeCard { service: DEFAULT_SERVICE.to_string(), handle, key }), relay })
    }

    /// Opens a wake for `handle` and checks it against repeats and the clock. Refusals count
    /// in [`WakeStatus::refused`].
    pub fn open_wake(&self, handle: &WakeHandle, payload: &weirdnet_push::Payload96) -> Result<Wake> {
        let r = self.open_inner(handle, payload);
        if r.is_err() {
            *lock(&self.refused) += 1;
        } else {
            let mut recent = lock(&self.recent);
            recent.push_back(crate::history::now_ms());
            while recent.len() > 16 {
                recent.pop_front();
            }
        }
        r
    }

    fn open_inner(&self, handle: &WakeHandle, payload: &weirdnet_push::Payload96) -> Result<Wake> {
        let keys = lock(&self.keys);
        let Some(k) = keys.as_ref().filter(|_| self.settings().push) else { bail!("push is off on this device") };
        if &k.handle != handle {
            bail!("a wake for another handle");
        }
        let w = k.key.open(handle, payload).map_err(|e| anyhow!("the wake does not open: {e}"))?;
        lock(&self.guard).check_now(&w).map_err(|e| anyhow!("refused: {e}"))?;
        Ok(w)
    }

    /// What the shells show (the relay connection is filled in by the core).
    pub fn status(&self, relay_connected: bool) -> WakeStatus {
        WakeStatus {
            settings: self.settings().into(),
            relay_connected,
            last_wake_ms: lock(&self.recent).back().copied(),
            refused: *lock(&self.refused),
        }
    }
}

/// Seals a wake for another device's card.
pub fn seal_for(card: &WakeCard, reason: Reason) -> Result<weirdnet_push::Payload96> {
    card.seal(reason).map_err(|e| anyhow!("sealing the wake: {e}"))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn from_hex32(s: &str) -> Result<[u8; 32]> {
    if s.len() != 64 || !s.is_ascii() {
        bail!("wake.key: not a 32-byte key");
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| anyhow!("wake.key: not hex"))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card_of(w: &Wakes) -> WakeCard {
        w.card(None).unwrap().card.unwrap()
    }

    #[test]
    fn push_is_on_and_relay_wake_off_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let w = Wakes::open(tmp.path()).unwrap();
        assert_eq!(w.settings(), WakeSettings { push: true, relay_wake: false });
        assert!(!w.settings().relay_active());
        // An old or partial file keeps the defaults for what it lacks.
        std::fs::write(tmp.path().join(SETTINGS_FILE), b"{}").unwrap();
        assert_eq!(Wakes::open(tmp.path()).unwrap().settings(), WakeSettings::default());
    }

    #[test]
    fn a_sealed_wake_opens_once_and_only_for_this_handle() {
        let tmp = tempfile::tempdir().unwrap();
        let w = Wakes::open(tmp.path()).unwrap();
        let card = card_of(&w);
        let p = seal_for(&card, Reason::MessageWaiting).unwrap();
        assert_eq!(w.open_wake(&card.handle, &p).unwrap().reason, Reason::MessageWaiting);
        assert!(w.open_wake(&card.handle, &p).is_err(), "a repeat is refused");
        assert!(w.open_wake(&WakeHandle::generate(), &p).is_err(), "another handle is refused");
        // Sealed for another device's key: does not open here.
        let other = Wakes::open(&tmp.path().join("other")).unwrap();
        let theirs = WakeCard { handle: card.handle.clone(), ..card_of(&other) };
        assert!(w.open_wake(&card.handle, &seal_for(&theirs, Reason::MessageWaiting).unwrap()).is_err());
        // Too old.
        let key = weirdnet_push::wake::seal_at(&card.public_key().unwrap(), &card.handle, Reason::ConnectionRequested, 1_000).unwrap();
        assert!(w.open_wake(&card.handle, &key).is_err(), "a stale wake is refused");
        assert_eq!(w.status(false).refused, 4);
        assert!(w.status(false).last_wake_ms.is_some());
    }

    #[test]
    fn the_key_survives_a_restart_and_push_off_forgets_it() {
        let tmp = tempfile::tempdir().unwrap();
        let w = Wakes::open(tmp.path()).unwrap();
        let card = card_of(&w);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(tmp.path().join(KEY_FILE)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let again = Wakes::open(tmp.path()).unwrap();
        assert_eq!(card_of(&again), card, "same key and handle after a restart");
        let p = seal_for(&card, Reason::MessageWaiting).unwrap();

        again.set(WakeSettings { push: false, relay_wake: true }).unwrap();
        assert!(!tmp.path().join(KEY_FILE).exists());
        assert_eq!(again.card(Some("/memory/1/p2p/x")).unwrap(), DeviceWake { card: None, relay: None }, "withdrawn");
        assert!(again.open_wake(&card.handle, &p).is_err(), "push off: nothing opens");
        assert!(!again.settings().relay_active(), "relay wake needs push");

        again.set(WakeSettings { push: true, relay_wake: true }).unwrap();
        let fresh = again.card(Some("/memory/1/p2p/x")).unwrap();
        assert_ne!(fresh.card.as_ref().unwrap().handle, card.handle, "a new handle after push comes back");
        assert_eq!(fresh.relay.as_deref(), Some("/memory/1/p2p/x"));
        assert!(again.open_wake(&card.handle, &p).is_err(), "the old handle no longer opens");
        assert_eq!(Wakes::open(tmp.path()).unwrap().settings(), WakeSettings { push: true, relay_wake: true });
    }
}
