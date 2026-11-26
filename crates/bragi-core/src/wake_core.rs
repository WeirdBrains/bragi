//! The core side of push wake-ups (B4; see [`crate::wake`]): publishing this device's card,
//! the relay wake connection, waking another linked device, and the state the shells draw.

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::watch;
use weirdnet_net::{Limits, Multiaddr, PeerId};
use weirdnet_push::Reason;
use weirdnet_push::relaywake::{self, Delivery, FromRelay, Subscription};

use crate::core::{AppState, Core, Transport};
use crate::error::{CoreError, ErrorKind, Result};
use crate::identity::{Identity, SyncNode, WriteError};
use crate::wake::{DeviceWake, WAKE_TYPE, WakeSettings, WakeStatus, Wakes, seal_for};

/// One of the user's other devices that can be woken.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WakeableDevice {
    pub peer_id: String,
    pub label: String,
    /// It keeps a wake connection to a relay, so [`Core::wake_device`] can reach it.
    pub through_relay: bool,
}

/// Parses `/…/p2p/PEER` into the peer and the address to dial it at.
pub(crate) fn relay_target(addr: &str) -> Option<(PeerId, Vec<Multiaddr>)> {
    let (base, peer) = addr.rsplit_once("/p2p/")?;
    let peer = PeerId::from_str(peer).ok()?;
    let a = Multiaddr::from_str(base).ok().map(|a| vec![a]).unwrap_or_default();
    Some((peer, a))
}

/// The home relay this device's wake connection goes to: the configured one, else (on the
/// real network) the WeirdBrains relay.
pub(crate) fn home_relay(wake_relay: &Option<String>, transport: Transport) -> Option<String> {
    wake_relay.clone().or_else(|| match transport {
        Transport::Internet => weirdnet_device_link::KNOWN_RELAYS.first().map(|s| s.to_string()),
        Transport::Memory => None,
    })
}

pub(crate) fn status_of(wakes: &Wakes, up: &AtomicBool) -> WakeStatus {
    wakes.status(up.load(Ordering::Relaxed))
}

/// Writes this device's card ([`DeviceWake`]) into the wake records when it changed. Before
/// the first key rotation with other devices it can't yet; the next sync pass tries again.
pub(crate) fn publish_card(identity: &Identity, wakes: &Wakes, relay: Option<&str>) {
    let card = match wakes.card(relay) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("the wake card: {e}");
            return;
        }
    };
    let me = identity.peer_id().to_string();
    let Ok(value) = serde_json::to_vec(&card) else { return };
    let current = identity.records(WAKE_TYPE).into_iter().find(|(k, _)| *k == me).map(|(_, v)| v);
    if current.as_deref() == Some(value.as_slice()) || (current.is_none() && card.card.is_none()) {
        return;
    }
    match identity.write_record(WAKE_TYPE, &me, &value) {
        Ok(()) | Err(WriteError::NoKeyYet) => {}
        Err(WriteError::Other(e)) => tracing::warn!("publishing the wake card: {e}"),
    }
}

/// The other active full devices' cards (service devices never share one).
pub(crate) fn device_cards(identity: &Identity) -> Vec<(String, String, DeviceWake)> {
    let me = identity.peer_id().to_string();
    let roster = identity.roster();
    identity
        .records(WAKE_TYPE)
        .into_iter()
        .filter(|(peer, _)| *peer != me && !identity.is_service(peer))
        .filter_map(|(peer, v)| {
            let cert = roster.active().find(|c| c.peer_id == peer)?.clone();
            let w: DeviceWake = serde_json::from_slice(&v).ok()?;
            w.card.as_ref()?;
            Some((peer, cert.label, w))
        })
        .collect()
}

/// Keeps the relay wake connection while [`WakeSettings::relay_active`], reconnecting with a
/// backoff; every valid wake runs `on_wake`. Restarts when the settings change.
pub(crate) async fn relay_loop(
    node: Arc<SyncNode>,
    wakes: Arc<Wakes>,
    relay: Option<String>,
    up: Arc<AtomicBool>,
    state: watch::Sender<AppState>,
    mut changed: watch::Receiver<u64>,
    on_wake: Arc<dyn Fn() + Send + Sync>,
) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let target = relay.as_deref().and_then(relay_target);
        let handle = if wakes.settings().relay_active() { wakes.handle().ok().flatten() } else { None };
        let (Some((peer, addrs)), Some(handle)) = (target, handle) else {
            set_up(&up, &state, &wakes, false);
            if changed.changed().await.is_err() {
                return;
            }
            continue;
        };
        let sub = async {
            let s = node.open_stream_async(peer, &addrs, relaywake::PROTOCOL, Limits::default()).await.map_err(|e| e.to_string())?;
            Subscription::start(s, std::slice::from_ref(&handle)).await.map_err(|e| e.to_string())
        };
        let mut sub = match tokio::time::timeout(Duration::from_secs(30), sub).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                tracing::debug!("relay wake: {e}");
                set_up(&up, &state, &wakes, false);
                wait_or_change(&mut changed, backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(60));
                continue;
            }
            Err(_) => {
                set_up(&up, &state, &wakes, false);
                wait_or_change(&mut changed, backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(60));
                continue;
            }
        };
        backoff = Duration::from_secs(1);
        set_up(&up, &state, &wakes, true);
        let idle = Duration::from_secs(u64::from(sub.keepalive_s.max(1)) * 2);
        loop {
            tokio::select! {
                r = changed.changed() => {
                    if r.is_err() { return; }
                    break;
                }
                f = tokio::time::timeout(idle, sub.next_frame()) => match f {
                    Ok(Ok(Some(FromRelay::Wake { handle: h, payload }))) => {
                        match wakes.open_wake(&h, &payload) {
                            Ok(w) => {
                                tracing::debug!(reason = w.reason.code(), "woken through the relay");
                                on_wake();
                            }
                            Err(e) => tracing::debug!("relay wake refused: {e}"),
                        }
                        state.send_modify(|s| s.wake = status_of(&wakes, &up));
                    }
                    Ok(Ok(Some(_))) => {}
                    _ => break,
                },
            }
        }
        set_up(&up, &state, &wakes, false);
    }
}

fn set_up(up: &AtomicBool, state: &watch::Sender<AppState>, wakes: &Wakes, v: bool) {
    up.store(v, Ordering::Relaxed);
    let st = wakes.status(v);
    state.send_if_modified(|s| {
        let changed = s.wake != st;
        s.wake = st.clone();
        changed
    });
}

async fn wait_or_change(changed: &mut watch::Receiver<u64>, d: Duration) {
    let _ = tokio::time::timeout(d, changed.changed()).await;
}

impl Core {
    /// This device's wake settings.
    pub fn wake_settings(&self) -> WakeSettings {
        self.inner.wakes.settings()
    }

    /// Changes them: push off withdraws this device's card and forgets its handle; relay wake
    /// on (with push) keeps a wake connection to the home relay (see [`crate::wake::RELAY_WAKE_COST`]).
    pub fn set_wake_settings(&self, s: WakeSettings) -> Result<()> {
        self.inner.wakes.set(s).map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?;
        self.inner.wake_gen.send_modify(|g| *g += 1);
        if let Ok(net) = self.net() {
            publish_card(&net.identity, &self.inner.wakes, self.wake_relay().as_deref());
            net.identity.refresh_wake_cards();
        }
        self.inner.state.send_modify(|st| st.wake = status_of(&self.inner.wakes, &self.inner.wake_up));
        Ok(())
    }

    /// What the shells show about wake-ups.
    pub fn wake_status(&self) -> WakeStatus {
        status_of(&self.inner.wakes, &self.inner.wake_up)
    }

    /// The home relay of this device's wake connection.
    pub fn wake_relay(&self) -> Option<String> {
        home_relay(&self.inner.cfg.wake_relay, self.inner.cfg.transport)
    }

    /// The user's other devices that shared a wake card.
    pub fn wakeable_devices(&self) -> Vec<WakeableDevice> {
        let Ok(net) = self.net() else { return Vec::new() };
        device_cards(&net.identity)
            .into_iter()
            .map(|(peer_id, label, w)| WakeableDevice { peer_id, label, through_relay: w.relay.is_some() })
            .collect()
    }

    /// Wakes another of the user's devices through the relay it keeps a wake connection to,
    /// so it syncs. A sealed 96-byte wake; the relay sees only the handle.
    pub async fn wake_device(&self, peer: &str, reason: Reason) -> Result<Delivery> {
        let net = self.net()?;
        let (_, _, w) = device_cards(&net.identity)
            .into_iter()
            .find(|(p, _, _)| p == peer)
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "that device shares no wake card"))?;
        let card = w.card.ok_or_else(|| CoreError::new(ErrorKind::NotFound, "that device turned push off"))?;
        let relay = w.relay.ok_or_else(|| CoreError::new(ErrorKind::Unavailable, "that device has no relay wake connection"))?;
        let (rpeer, addrs) = relay_target(&relay).ok_or_else(|| CoreError::new(ErrorKind::Other, "a bad relay address"))?;
        let payload = seal_for(&card, reason).map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?;
        let io = net
            .node
            .open_stream_async(rpeer, &addrs, relaywake::PROTOCOL, Limits::default())
            .await
            .map_err(|e| CoreError::new(ErrorKind::Unavailable, format!("reaching the relay: {e}")))?;
        tokio::time::timeout(Duration::from_secs(30), relaywake::deliver(io, &card.handle, payload))
            .await
            .map_err(|_| CoreError::new(ErrorKind::Unavailable, "the relay did not answer"))?
            .map_err(|e| CoreError::new(ErrorKind::Unavailable, format!("the relay: {e}")))
    }
}
