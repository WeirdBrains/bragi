//! Weirdnet links: the node addresses this device dials (`/ip4/…/tcp/4848/p2p/PEER`), the ones
//! link codes brought and the ones the user added under Link a device. Each can be renamed, have
//! its address edited (tested by dialing it first) or be deleted (forgotten and no longer dialed).
//!
//! The addresses stay in [`crate::PEERS_FILE`] as before; their names are in `<dir>/links.json`,
//! keyed by address. Addresses from the app's config (the demo, tests) can be renamed but not
//! edited or deleted. No session is bound to one link: sessions send to the whole Weirdnet pool,
//! so deleting a link never touches a session or its history.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use weirdnet_net::{Multiaddr, PeerHint};

use crate::endpoint::{clean_name, write_file};
use crate::error::{CoreError, ErrorKind, Result};

pub const LINKS_FILE: &str = "links.json";
/// How long a test connection may take.
const TEST_TIMEOUT: Duration = Duration::from_secs(20);

/// One node address as the shells draw it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkState {
    /// The address; also what the actions take to name it.
    pub address: String,
    pub name: String,
    /// The node's Weirdnet peer id (the `/p2p/` part).
    pub peer_id: String,
    /// This device has a connection to that node right now.
    pub connected: bool,
    /// From the app's config: it can be renamed only.
    pub fixed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    names: BTreeMap<String, String>,
}

/// The link names in a data folder.
pub(crate) struct LinkNames {
    path: PathBuf,
    stored: Stored,
}

fn bad(msg: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::BadRequest, msg)
}

/// Parses an address that ends in `/p2p/PEER`; returns its peer id.
pub(crate) fn parse_address(address: &str) -> Result<(String, Multiaddr)> {
    let a = address.trim();
    let hint = PeerHint::from_addr(a).map_err(|e| bad(format!("address `{a}`: {e}")))?;
    let m: Multiaddr = a.parse().map_err(|e| bad(format!("address `{a}`: {e}")))?;
    Ok((hint.peer_id, m))
}

/// The name a link has until the user gives it one.
fn default_name(peer_id: &str) -> String {
    let tail: String = peer_id.chars().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect();
    format!("Node …{tail}")
}

impl LinkNames {
    pub(crate) fn open(dir: &Path) -> Self {
        let path = dir.join(LINKS_FILE);
        let stored = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { path, stored }
    }

    fn save(&self) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.stored).map_err(|e| CoreError::new(ErrorKind::Storage, e.to_string()))?;
        write_file(&self.path, &bytes, false)
    }

    pub(crate) fn name(&self, address: &str, peer_id: &str) -> String {
        self.stored.names.get(address).cloned().unwrap_or_else(|| default_name(peer_id))
    }

    pub(crate) fn set(&mut self, address: &str, name: Option<String>) -> Result<()> {
        match name {
            Some(n) => self.stored.names.insert(address.to_string(), n),
            None => self.stored.names.remove(address),
        };
        self.save()
    }

    pub(crate) fn moved(&mut self, from: &str, to: &str) -> Result<()> {
        if let Some(n) = self.stored.names.remove(from) {
            self.stored.names.insert(to.to_string(), n);
        }
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_their_address() {
        let tmp = tempfile::tempdir().unwrap();
        let mut n = LinkNames::open(tmp.path());
        assert_eq!(n.name("/memory/1/p2p/x", "12D3KooWabcdef123456"), "Node …123456");
        n.set("/memory/1/p2p/x", Some("Desk".into())).unwrap();
        n.moved("/memory/1/p2p/x", "/memory/2/p2p/x").unwrap();
        let again = LinkNames::open(tmp.path());
        assert_eq!(again.name("/memory/2/p2p/x", "p"), "Desk");
        assert_eq!(again.name("/memory/1/p2p/x", "p…abc"), "Node …p…abc");
        assert!(parse_address("/ip4/192.0.2.1/tcp/4848").is_err(), "an address needs its /p2p/ part");
    }
}

// ---- the core's link actions ------------------------------------------------------------------

impl crate::Core {
    /// The node addresses this device dials, with their names and whether each is connected.
    pub fn links(&self) -> Vec<LinkState> {
        let connected: Vec<String> = self.net().map(|n| n.node.status().connected).unwrap_or_default();
        let names = crate::core::lock(&self.inner.links);
        let fixed = &self.inner.cfg.peers;
        crate::core::lock(&self.inner.peers)
            .iter()
            .filter_map(|a| {
                let (peer, _) = parse_address(a).ok()?;
                Some(LinkState {
                    address: a.clone(),
                    name: names.name(a, &peer),
                    connected: connected.contains(&peer),
                    peer_id: peer,
                    fixed: fixed.contains(a),
                })
            })
            .collect()
    }

    pub(crate) fn publish_links(&self) {
        let list = self.links();
        self.inner.state.send_if_modified(|s| {
            let changed = s.links != list;
            if changed {
                s.links = list;
            }
            changed
        });
    }

    /// A test connection: dials `address` (it must end in `/p2p/PEER`) and checks that the node
    /// there is that peer. Saves nothing. Needs this device to be online.
    pub async fn test_link(&self, address: &str) -> Result<String> {
        let (peer, m) = parse_address(address)?;
        let net = self.net().map_err(|_| bad("link this device to your Weirdnet first, so Bragi can test the address"))?;
        let node = net.node.clone();
        let got = tokio::time::timeout(TEST_TIMEOUT, node.dial_async(m))
            .await
            .map_err(|_| CoreError::new(ErrorKind::Unavailable, format!("no node answered at {} in time", address.trim())))?
            .map_err(|e| CoreError::new(ErrorKind::Unavailable, format!("can't reach {}: {e}", address.trim())))?;
        if got.to_string() != peer {
            return Err(bad(format!("a different node answered there ({got})")));
        }
        Ok(peer)
    }

    /// Adds a node address after a test connection, with a name (its peer id's end when empty).
    pub async fn add_link(&self, name: &str, address: &str) -> Result<LinkState> {
        let address = address.trim().to_string();
        self.test_link(&address).await?;
        self.add_peer(&address)?;
        if !name.trim().is_empty() {
            let n = clean_name(name, "link")?;
            crate::core::lock(&self.inner.links).set(&address, Some(n))?;
        }
        self.publish_links();
        self.links().into_iter().find(|l| l.address == address).ok_or_else(|| bad("no such link"))
    }

    /// Renames a link (an empty name gives back the default one).
    pub fn rename_link(&self, address: &str, name: &str) -> Result<()> {
        if !self.links().iter().any(|l| l.address == address) {
            return Err(bad("no such link"));
        }
        let n = if name.trim().is_empty() { None } else { Some(clean_name(name, "link")?) };
        crate::core::lock(&self.inner.links).set(address, n)?;
        self.publish_links();
        Ok(())
    }

    /// Changes a link's address, after a test connection to the new one; nothing changes if the
    /// test fails. Its name moves with it.
    pub async fn edit_link(&self, address: &str, new_address: &str) -> Result<LinkState> {
        let new_address = new_address.trim().to_string();
        let old = self.links().into_iter().find(|l| l.address == address).ok_or_else(|| bad("no such link"))?;
        if old.fixed {
            return Err(bad("that address comes from Bragi's config; it can be renamed only"));
        }
        if new_address == old.address {
            return Ok(old);
        }
        if self.links().iter().any(|l| l.address == new_address) {
            return Err(bad("that address is already a link"));
        }
        self.test_link(&new_address).await?;
        {
            let mut p = crate::core::lock(&self.inner.peers);
            if let Some(slot) = p.iter_mut().find(|a| **a == old.address) {
                *slot = new_address.clone();
            }
        }
        self.save_peers();
        crate::core::lock(&self.inner.links).moved(&old.address, &new_address)?;
        self.forget_peer_if_unused(&old.peer_id);
        let _ = self.add_peer(&new_address);
        self.publish_links();
        self.links().into_iter().find(|l| l.address == new_address).ok_or_else(|| bad("no such link"))
    }

    /// Deletes a link: the address is forgotten and no longer dialed. No session or history
    /// depends on one link, so nothing else changes. Returns how many sessions were bound to it
    /// (always 0: sessions use the whole pool).
    pub fn delete_link(&self, address: &str) -> Result<usize> {
        let old = self.links().into_iter().find(|l| l.address == address).ok_or_else(|| bad("no such link"))?;
        if old.fixed {
            return Err(bad("that address comes from Bragi's config; it can be renamed only"));
        }
        crate::core::lock(&self.inner.peers).retain(|a| *a != old.address);
        self.save_peers();
        crate::core::lock(&self.inner.links).set(&old.address, None)?;
        self.forget_peer_if_unused(&old.peer_id);
        self.publish_links();
        Ok(0)
    }

    /// Stops the node dialing `peer` when no link has an address for it any more.
    fn forget_peer_if_unused(&self, peer: &str) {
        if self.links().iter().any(|l| l.peer_id == peer) {
            return;
        }
        if let (Ok(net), Ok(id)) = (self.net(), peer.parse()) {
            let node = net.node.clone();
            self.inner.handle.spawn_blocking(move || {
                if let Err(e) = node.remove_peer(id) {
                    tracing::warn!("forgetting a node: {e}");
                }
            });
        }
    }
}
