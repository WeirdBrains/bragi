//! A stand-in home relay for the relay wake path (`/weirdnet/wake/1`) on the memory transport,
//! for tests and the demo profile only. It does what weirdnet-relay's wake service does for
//! one process: devices subscribe their handles on a stream they keep open, senders deliver a
//! sealed payload for a handle, and the relay passes it on (or answers `offline`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow};
use weirdnet_net::{Handler, Keypair, Limits, Node, PeerId, WEIRDNET};
use weirdnet_push::relaywake::{self, Delivery, FromRelay, ToRelay};
use weirdnet_push::{Payload96, WakeHandle};

struct NoRequests;

impl Handler<Vec<u8>, Vec<u8>> for NoRequests {
    fn request(&self, _from: &PeerId, _req: Vec<u8>) -> Vec<u8> {
        Vec::new()
    }
}

type Subs = Arc<Mutex<HashMap<WakeHandle, tokio::sync::mpsc::Sender<(WakeHandle, Payload96)>>>>;

/// The stand-in relay.
pub struct WakeRelay {
    node: Arc<Node<Vec<u8>, Vec<u8>>>,
    subs: Subs,
    delivered: Arc<Mutex<u64>>,
}

impl WakeRelay {
    /// Starts it on the memory transport; keepalives every `keepalive`.
    pub async fn start(keepalive: Duration) -> Result<Self> {
        let node = tokio::task::spawn_blocking(|| {
            Node::start(&WEIRDNET, Keypair::generate_ed25519(), weirdnet_sync::node_config(true), Arc::new(NoRequests))
        })
        .await
        .map_err(|e| anyhow!("{e}"))?
        .map_err(|e| anyhow!("starting the wake relay: {e}"))?;
        let node = Arc::new(node);
        let mut incoming = node.accept_streams(relaywake::PROTOCOL, Limits::default()).map_err(|e| anyhow!("{e}"))?;
        let subs: Subs = Arc::default();
        let delivered = Arc::new(Mutex::new(0));
        let (s, d) = (subs.clone(), delivered.clone());
        let rt = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            while let Some(stream) = incoming.accept() {
                let (s, d) = (s.clone(), d.clone());
                rt.spawn(serve(stream, s, d, keepalive));
            }
        });
        Ok(Self { node, subs, delivered })
    }

    /// Where devices dial it: `/memory/N/p2p/PEER`.
    pub fn address(&self) -> String {
        for _ in 0..200 {
            if let Some(a) = self.node.status().listening.first() {
                return format!("{}/p2p/{}", a.split("/p2p/").next().unwrap_or(a), self.node.peer_id());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        String::new()
    }

    /// How many handles are subscribed now.
    pub fn subscribed(&self) -> usize {
        self.subs.lock().unwrap_or_else(|e| e.into_inner()).values().filter(|t| !t.is_closed()).count()
    }

    /// How many wakes it passed on.
    pub fn delivered(&self) -> u64 {
        *self.delivered.lock().unwrap_or_else(|e| e.into_inner())
    }
}

async fn serve(mut io: weirdnet_net::Stream, subs: Subs, delivered: Arc<Mutex<u64>>, keepalive: Duration) {
    match relaywake::read::<_, ToRelay>(&mut io).await {
        Ok(Some(ToRelay::Subscribe { handles })) => {
            let (tx, mut rx) = tokio::sync::mpsc::channel(8);
            {
                let mut s = subs.lock().unwrap_or_else(|e| e.into_inner());
                for h in &handles {
                    s.insert(h.clone(), tx.clone());
                }
            }
            let ka = keepalive.as_secs().max(1) as u32;
            if relaywake::write(&mut io, &FromRelay::Subscribed { handles: handles.len(), keepalive_s: ka }).await.is_err() {
                return;
            }
            let mut tick = tokio::time::interval(keepalive);
            loop {
                let frame = tokio::select! {
                    p = rx.recv() => match p {
                        Some((handle, payload)) => FromRelay::Wake { handle, payload },
                        None => return,
                    },
                    _ = tick.tick() => FromRelay::Keepalive,
                };
                if relaywake::write(&mut io, &frame).await.is_err() {
                    let mut s = subs.lock().unwrap_or_else(|e| e.into_inner());
                    s.retain(|_, t| !t.same_channel(&tx));
                    return;
                }
            }
        }
        Ok(Some(ToRelay::Deliver { handle, payload })) => {
            let tx = subs.lock().unwrap_or_else(|e| e.into_inner()).get(&handle).cloned();
            let status = match tx {
                Some(tx) if tx.send((handle, payload)).await.is_ok() => {
                    *delivered.lock().unwrap_or_else(|e| e.into_inner()) += 1;
                    Delivery::Delivered
                }
                _ => Delivery::Offline,
            };
            let _ = relaywake::write(&mut io, &FromRelay::Delivered { status }).await;
            let _ = futures::AsyncWriteExt::close(&mut io).await;
        }
        _ => {}
    }
}
