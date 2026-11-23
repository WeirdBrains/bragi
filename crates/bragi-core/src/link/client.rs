//! The phone side of the link: pairing from a QR code or typed code, the paired desktops
//! (`<profile>/linker/client.json`, 0600, with this phone's own link key in `client.key`), and
//! the code tools as calls to a desktop, directly or through Weirdnet.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use weirdnet_net::{AppError, AppRequest, AppResponse, Multiaddr, NodeConfig, PeerId};

use crate::core::lock;
use crate::error::{CoreError, ErrorKind, Result};
use crate::history::now_ms;

use super::proto::*;
use super::server::{LinkNode, NoRequests, load_or_make_key};

/// How long to wait for a desktop's answer to start (a shell's output may then go on).
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a direct try gets before the Weirdnet route is tried.
const DIRECT_TRY: Duration = Duration::from_secs(6);

/// Which way calls go to a desktop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
    /// Directly over the LAN or VPN first, then through Weirdnet.
    #[default]
    Auto,
    /// Only directly; never through a relay.
    Direct,
    /// Through Weirdnet first, then directly.
    Weirdnet,
}

impl Route {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Route::Auto),
            "direct" | "lan" => Some(Route::Direct),
            "weirdnet" | "relay" => Some(Route::Weirdnet),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Route::Auto => "auto",
            Route::Direct => "direct",
            Route::Weirdnet => "weirdnet",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Desktop {
    /// Its link peer id: the key pinned at pairing.
    peer: String,
    label: String,
    direct: Vec<String>,
    relays: Vec<u8>,
    token: String,
    device_id: String,
    roots: Vec<RootInfo>,
    confirm_shell: bool,
    paired_ms: i64,
}

/// A paired desktop, for showing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopView {
    /// Its peer id (what the other calls take).
    pub id: String,
    pub label: String,
    pub roots: Vec<RootInfo>,
    pub confirm_shell: bool,
    pub paired_ms: i64,
    /// Reachable directly at these addresses (as last heard).
    pub direct: Vec<String>,
    /// Reachable through Weirdnet (as last heard).
    pub weirdnet: bool,
}

impl From<&Desktop> for DesktopView {
    fn from(d: &Desktop) -> Self {
        DesktopView {
            id: d.peer.clone(),
            label: d.label.clone(),
            roots: d.roots.clone(),
            confirm_shell: d.confirm_shell,
            paired_ms: d.paired_ms,
            direct: d.direct.clone(),
            weirdnet: !d.relays.is_empty(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ClientFile {
    #[serde(default)]
    route: Route,
    #[serde(default)]
    desktops: Vec<Desktop>,
}

pub struct LinkClient {
    dir: PathBuf,
    memory: bool,
    node: tokio::sync::Mutex<Option<Arc<LinkNode>>>,
    file: Mutex<ClientFile>,
}

fn unreachable(e: impl std::fmt::Display) -> CoreError {
    CoreError::new(ErrorKind::Unavailable, format!("couldn't reach the desktop: {e}"))
}

fn app_err(e: AppError) -> CoreError {
    match e {
        AppError::Unreachable(m) => unreachable(m),
        other => CoreError::new(ErrorKind::Unavailable, other.to_string()),
    }
}

impl LinkClient {
    /// `dir` is `<profile>/linker`. `memory`: the in-memory transport (tests).
    pub fn open(dir: impl Into<PathBuf>, memory: bool) -> Self {
        let dir = dir.into();
        let file = std::fs::read(dir.join("client.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        LinkClient { dir, memory, node: tokio::sync::Mutex::new(None), file: Mutex::new(file) }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn save(&self, f: &ClientFile) -> Result<()> {
        super::server::private_dir(&self.dir)?;
        let bytes = serde_json::to_vec_pretty(f).map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?;
        crate::identity::write_private_file(&self.dir.join("client.json"), &bytes)?;
        Ok(())
    }

    async fn node(&self) -> Result<Arc<LinkNode>> {
        let mut n = self.node.lock().await;
        if let Some(node) = n.as_ref().filter(|n| n.running()) {
            return Ok(node.clone());
        }
        let key = load_or_make_key(&self.dir.join("client.key"))?;
        let mut cfg = if self.memory { weirdnet_sync::node_config(true) } else { NodeConfig::default() };
        if !self.memory {
            // Outbound only: nothing listens on the phone, nothing is announced.
            cfg.listen = Vec::new();
            cfg.mdns = false;
            cfg.discovery = weirdnet_net::DiscoveryMode::Disabled;
        }
        cfg.label = "bragi-link-client".into();
        cfg.version = format!("bragi-link/{}", env!("CARGO_PKG_VERSION"));
        cfg.app_idle_timeout = Duration::from_secs(120);
        let node = tokio::task::spawn_blocking(move || LinkNode::start(&weirdnet_net::WEIRDNET, key, cfg, Arc::new(NoRequests)))
            .await
            .map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?
            .map_err(|e| CoreError::new(ErrorKind::Unavailable, format!("starting the link: {e}")))?;
        let node = Arc::new(node);
        *n = Some(node.clone());
        Ok(node)
    }

    /// The paired desktops.
    pub fn desktops(&self) -> Vec<DesktopView> {
        lock(&self.file).desktops.iter().map(DesktopView::from).collect()
    }

    pub fn route(&self) -> Route {
        lock(&self.file).route
    }

    pub fn set_route(&self, route: Route) -> Result<()> {
        let mut f = lock(&self.file);
        f.route = route;
        self.save(&f)
    }

    fn desktop(&self, id: &str) -> Result<Desktop> {
        lock(&self.file)
            .desktops
            .iter()
            .find(|d| d.peer == id)
            .cloned()
            .ok_or_else(|| CoreError::new(ErrorKind::NotPaired, "that desktop isn't paired"))
    }

    /// Pairs with the desktop showing `code`: the text of its QR code
    /// (`weirdnet-bragi://desktop?…`) or the code typed from its screen. `label` is what the
    /// desktop will list this phone as.
    pub async fn pair(&self, code: &str, label: &str) -> Result<DesktopView> {
        let node = self.node().await?;
        let code = code.trim();
        let (peer, addrs, secret) = if code.starts_with(LINK_PREFIX) {
            let link = PairLink::parse(code)?;
            if link.expires_ms > 0 && link.expires_ms + 60_000 < now_ms() {
                return Err(CoreError::new(ErrorKind::Pairing, "that pairing code expired; show a new one on the desktop"));
            }
            let peer: PeerId = link.peer.parse().map_err(|_| CoreError::new(ErrorKind::Pairing, "bad peer id"))?;
            let addrs = self.ordered(&link.peer, &link.direct, &link.relays);
            (peer, addrs, link.secret)
        } else {
            let short = ShortCode::parse(code)?;
            let addr: Multiaddr =
                format!("/ip4/{}/tcp/{}", short.ip, short.port).parse().map_err(|_| CoreError::new(ErrorKind::Pairing, "bad address"))?;
            let peer = tokio::time::timeout(CALL_TIMEOUT, node.dial_async(addr.clone()))
                .await
                .map_err(|_| unreachable("timed out"))?
                .map_err(unreachable)?;
            // The key Noise just checked must be the one the code names, before the secret goes.
            if fingerprint(&peer) != short.fingerprint {
                return Err(CoreError::new(ErrorKind::Pairing, "the computer at that address isn't the desktop that showed this code"));
            }
            let full: Multiaddr = format!("{addr}/p2p/{peer}").parse().map_err(|_| CoreError::new(ErrorKind::Pairing, "bad address"))?;
            (peer, vec![vec![full]], short.secret_hex())
        };
        let body = serde_json::to_vec(&PairRequest { secret, label: label.to_string() }).unwrap_or_default();
        let resp = self.send(&node, peer, &addrs, "POST", "/v1/pair", None, body).await?;
        let reply: PairReply = read_json(resp).await?;
        let d = Desktop {
            peer: peer.to_string(),
            label: reply.desktop.label.clone(),
            direct: reply.desktop.direct.clone(),
            relays: reply.desktop.relays.clone(),
            token: reply.token,
            device_id: reply.device_id,
            roots: reply.desktop.roots.clone(),
            confirm_shell: reply.desktop.confirm_shell,
            paired_ms: now_ms(),
        };
        let mut f = lock(&self.file);
        f.desktops.retain(|x| x.peer != d.peer);
        f.desktops.push(d.clone());
        self.save(&f)?;
        Ok(DesktopView::from(&d))
    }

    /// Forgets a desktop here, and asks it to drop this phone too (best effort).
    pub async fn forget(&self, id: &str) -> Result<()> {
        if let Ok(d) = self.desktop(id) {
            let _ = tokio::time::timeout(Duration::from_secs(5), self.call(&d, "/v1/unpair", Vec::new())).await;
        }
        let mut f = lock(&self.file);
        f.desktops.retain(|x| x.peer != id);
        self.save(&f)
    }

    /// The address groups to try, in order, for this phone's route setting.
    fn ordered(&self, peer: &str, direct: &[String], relays: &[u8]) -> Vec<Vec<Multiaddr>> {
        let parse = |v: Vec<String>| v.iter().filter_map(|a| a.parse().ok()).collect::<Vec<Multiaddr>>();
        let d = parse(route_addresses(peer, direct, &[]));
        let w = parse(route_addresses(peer, &[], relays));
        let groups = match self.route() {
            Route::Auto => vec![d, w],
            Route::Direct => vec![d],
            Route::Weirdnet => vec![w, d],
        };
        groups.into_iter().filter(|g| !g.is_empty()).collect()
    }

    #[allow(clippy::too_many_arguments)]
    async fn send(
        &self,
        node: &LinkNode,
        peer: PeerId,
        groups: &[Vec<Multiaddr>],
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Vec<u8>,
    ) -> Result<AppResponse> {
        if groups.is_empty() {
            return Err(unreachable(match self.route() {
                Route::Direct => "it has no LAN or VPN address, and the route is set to Direct only",
                _ => "no address for it; pair again",
            }));
        }
        let mut last = None;
        for (i, addrs) in groups.iter().enumerate() {
            let mut req = AppRequest::new(APP, method, path).header("content-type", "application/json").body(body.clone());
            if let Some(t) = token {
                req = req.header("authorization", &format!("Bearer {t}"));
            }
            // A direct try that isn't the last gets a short time, so Weirdnet gets its turn.
            let wait = if i + 1 < groups.len() && !addrs.iter().any(|a| is_circuit(&a.to_string())) { DIRECT_TRY } else { CALL_TIMEOUT };
            match node.app_call_via(peer, addrs, req, wait).await {
                Ok(r) => return check(r).await,
                Err(e @ (AppError::Unreachable(_) | AppError::TimedOut)) => last = Some(e),
                Err(e) => return Err(app_err(e)),
            }
        }
        Err(app_err(last.unwrap_or(AppError::TimedOut)))
    }

    async fn call(&self, d: &Desktop, path: &str, body: Vec<u8>) -> Result<AppResponse> {
        let node = self.node().await?;
        let peer: PeerId = d.peer.parse().map_err(|_| CoreError::new(ErrorKind::Storage, "bad peer id"))?;
        let groups = self.ordered(&d.peer, &d.direct, &d.relays);
        self.send(&node, peer, &groups, "POST", path, Some(&d.token), body).await
    }

    async fn post<Q: Serialize, R: for<'de> Deserialize<'de>>(&self, id: &str, path: &str, q: &Q) -> Result<R> {
        let d = self.desktop(id)?;
        let resp = self.call(&d, path, serde_json::to_vec(q).unwrap_or_default()).await?;
        read_json(resp).await
    }

    /// The desktop and its shared projects now; also refreshes its addresses here.
    pub async fn hello(&self, id: &str) -> Result<Hello> {
        let h: Hello = self.post(id, "/v1/hello", &serde_json::json!({})).await?;
        let mut f = lock(&self.file);
        if let Some(d) = f.desktops.iter_mut().find(|d| d.peer == id) {
            if !h.direct.is_empty() {
                d.direct = h.direct.clone();
            }
            if !h.relays.is_empty() {
                d.relays = h.relays.clone();
            }
            d.roots = h.roots.clone();
            d.confirm_shell = h.confirm_shell;
            d.label = h.label.clone();
            let all = f.clone();
            let _ = self.save(&all);
        }
        Ok(h)
    }

    pub async fn list(&self, id: &str, root: usize, path: &str) -> Result<Vec<Entry>> {
        let r: ListReply = self.post(id, "/v1/list", &PathRequest { root, path: path.into() }).await?;
        Ok(r.entries)
    }

    pub async fn read(&self, id: &str, root: usize, path: &str) -> Result<Option<String>> {
        let r: ReadReply = self.post(id, "/v1/read", &PathRequest { root, path: path.into() }).await?;
        Ok(r.text)
    }

    pub async fn gather(&self, id: &str, root: usize, targets: &[String]) -> Result<GatherReply> {
        self.post(id, "/v1/gather", &GatherRequest { root, targets: targets.to_vec() }).await
    }

    pub async fn apply(&self, id: &str, root: usize, changes: Vec<WireChange>) -> Result<AppliedReply> {
        self.post(id, "/v1/apply", &ApplyRequest { root, changes }).await
    }

    pub async fn undo(&self, id: &str, root: usize, force: bool) -> Result<AppliedReply> {
        self.post(id, "/v1/undo", &UndoRequest { root, force }).await
    }

    pub async fn project(&self, id: &str, root: usize, path: &str) -> Result<ProjectReply> {
        self.post(id, "/v1/project", &PathRequest { root, path: path.into() }).await
    }

    /// Runs a command on the desktop, handing each frame to `on` as it comes; returns the last
    /// ([`ShellFrame::Exit`] or [`ShellFrame::Denied`]). Dropping the future stops the command.
    pub async fn shell(&self, id: &str, req: ShellRequest, on: &mut (dyn FnMut(ShellFrame) + Send)) -> Result<ShellFrame> {
        let d = self.desktop(id)?;
        let resp = self.call(&d, "/v1/shell", serde_json::to_vec(&req).unwrap_or_default()).await?;
        let mut body = resp.body;
        let mut buf: Vec<u8> = Vec::new();
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(app_err)?;
            buf.extend_from_slice(&chunk);
            while let Some(nl) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=nl).collect();
                let Ok(frame) = serde_json::from_slice::<ShellFrame>(&line) else { continue };
                match frame {
                    ShellFrame::Alive => {}
                    f @ (ShellFrame::Exit { .. } | ShellFrame::Denied { .. }) => {
                        on(f.clone());
                        return Ok(f);
                    }
                    f => on(f),
                }
            }
        }
        Err(unreachable("the desktop closed the command before it ended"))
    }
}

async fn check(r: AppResponse) -> Result<AppResponse> {
    if (200..300).contains(&r.status) {
        return Ok(r);
    }
    let status = r.status;
    let body = r.body.collect().await.unwrap_or_default();
    Err(match serde_json::from_slice::<WireError>(&body) {
        Ok(w) => CoreError::new(w.kind, w.message),
        Err(_) => CoreError::new(ErrorKind::Other, format!("the desktop answered {status}")),
    })
}

async fn read_json<T: for<'de> Deserialize<'de>>(r: AppResponse) -> Result<T> {
    let body = r.body.collect().await.map_err(app_err)?;
    serde_json::from_slice(&body).map_err(|e| CoreError::new(ErrorKind::Other, format!("the desktop's answer: {e}")))
}
