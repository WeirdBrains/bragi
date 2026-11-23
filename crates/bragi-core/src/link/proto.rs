//! The desktop link's wire format: what a pairing QR code (or typed code) holds, and the JSON
//! bodies of each call. See `docs/desktop-link.md` for the protocol as a whole.

use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::code::ContextFile;
use crate::error::{CoreError, ErrorKind, Result};

/// The Weirdnet app name the desktop serves the link under (`/weirdnet/app/1`, app calls).
pub const APP: &str = "bragi-link";
/// The protocol version in pairing links and `hello`.
pub const VERSION: u32 = 1;
/// The QR code's URL prefix (the app's own scheme, never bare `weirdnet://`).
pub const LINK_PREFIX: &str = "weirdnet-bragi://desktop";

fn bad(msg: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::Pairing, msg)
}

/// What a pairing QR code carries: who the desktop is (its peer id, which is its public key, so
/// the phone pins it), where to reach it, and a one-time secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairLink {
    /// The desktop's link node peer id (`12D3Koo…`): the Ed25519 key Noise checks on every
    /// connection, directly or through a relay.
    pub peer: String,
    /// The one-time pairing secret, hex.
    pub secret: String,
    pub label: String,
    /// Direct addresses (LAN or VPN), `/ip4/…/tcp/…`, without `/p2p/`.
    pub direct: Vec<String>,
    /// Weirdnet relays the desktop holds a reservation on, as indexes into
    /// [`weirdnet_device_link::KNOWN_RELAYS`]: the route through Weirdnet.
    pub relays: Vec<u8>,
    /// When the code stops working (ms since the epoch).
    pub expires_ms: i64,
}

impl PairLink {
    /// `weirdnet-bragi://desktop?v=1&p=PEER&s=SECRET&n=LABEL&x=EXPIRES&a=ADDR…&r=RELAY…`
    pub fn to_url(&self) -> String {
        let mut s = format!("{LINK_PREFIX}?v={VERSION}&p={}&s={}&n={}&x={}", enc(&self.peer), enc(&self.secret), enc(&self.label), self.expires_ms);
        for a in &self.direct {
            s.push_str(&format!("&a={}", enc(a)));
        }
        for r in &self.relays {
            s.push_str(&format!("&r={r}"));
        }
        s
    }

    pub fn parse(url: &str) -> Result<Self> {
        let q = url.trim().strip_prefix(LINK_PREFIX).and_then(|r| r.strip_prefix('?')).ok_or_else(|| bad("not a Bragi desktop pairing code"))?;
        let mut out = PairLink { peer: String::new(), secret: String::new(), label: String::new(), direct: vec![], relays: vec![], expires_ms: 0 };
        let mut version = 0u32;
        for part in q.split('&') {
            let (k, v) = part.split_once('=').unwrap_or((part, ""));
            let v = dec(v)?;
            match k {
                "v" => version = v.parse().map_err(|_| bad("bad version"))?,
                "p" => out.peer = v,
                "s" => out.secret = v,
                "n" => out.label = v,
                "x" => out.expires_ms = v.parse().map_err(|_| bad("bad expiry"))?,
                "a" => out.direct.push(v),
                "r" => out.relays.push(v.parse().map_err(|_| bad("bad relay"))?),
                _ => {}
            }
        }
        if version != VERSION {
            return Err(bad(format!("this pairing code is version {version}; update Bragi on both devices")));
        }
        if out.peer.parse::<weirdnet_net::PeerId>().is_err() || out.secret.is_empty() {
            return Err(bad("the pairing code is incomplete"));
        }
        Ok(out)
    }

    /// Every address to try, `/p2p/PEER` included: direct ones, then relay circuits.
    pub fn addresses(&self) -> Vec<String> {
        route_addresses(&self.peer, &self.direct, &self.relays)
    }
}

/// Full multiaddrs for `peer`: the direct ones, then a circuit through each known relay.
pub fn route_addresses(peer: &str, direct: &[String], relays: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = direct.iter().map(|a| format!("{}/p2p/{peer}", a.trim_end_matches('/'))).collect();
    for r in relays {
        if let Some(relay) = weirdnet_device_link::KNOWN_RELAYS.get(*r as usize) {
            out.push(format!("{relay}/p2p-circuit/p2p/{peer}"));
        }
    }
    out
}

/// Whether a multiaddr goes through a relay (the Weirdnet route).
pub fn is_circuit(addr: &str) -> bool {
    addr.contains("/p2p-circuit")
}

/// The fallback to a QR code: a code to type, `XXXX-XXXX-…` (28 characters of Crockford base32),
/// for a desktop on the same LAN or VPN. It holds the desktop's IPv4 address and port, a
/// one-time 48-bit secret and the first 40 bits of the BLAKE3 hash of its peer id, which the
/// phone checks against the key Noise shows it before sending the secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortCode {
    pub ip: Ipv4Addr,
    pub port: u16,
    pub secret: [u8; 6],
    pub fingerprint: [u8; 5],
}

/// The first 5 bytes of BLAKE3 over the peer id's bytes.
pub fn fingerprint(peer: &weirdnet_net::PeerId) -> [u8; 5] {
    let h = blake3::hash(&peer.to_bytes());
    let mut f = [0u8; 5];
    f.copy_from_slice(&h.as_bytes()[..5]);
    f
}

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

impl ShortCode {
    pub fn encode(&self) -> String {
        let mut bytes = Vec::with_capacity(17);
        bytes.extend_from_slice(&self.ip.octets());
        bytes.extend_from_slice(&self.port.to_be_bytes());
        bytes.extend_from_slice(&self.secret);
        bytes.extend_from_slice(&self.fingerprint);
        let chars = base32(&bytes);
        chars.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("-")
    }

    pub fn parse(code: &str) -> Result<Self> {
        let clean: String = code.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
        if clean.len() != 28 {
            return Err(bad("a pairing code has 28 letters and digits"));
        }
        let b = unbase32(&clean)?;
        if b.len() < 17 {
            return Err(bad("the pairing code is incomplete"));
        }
        let mut secret = [0u8; 6];
        secret.copy_from_slice(&b[6..12]);
        let mut fingerprint = [0u8; 5];
        fingerprint.copy_from_slice(&b[12..17]);
        Ok(ShortCode { ip: Ipv4Addr::new(b[0], b[1], b[2], b[3]), port: u16::from_be_bytes([b[4], b[5]]), secret, fingerprint })
    }

    pub fn secret_hex(&self) -> String {
        hex(&self.secret)
    }
}

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in bytes {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(CROCKFORD[((buf >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(CROCKFORD[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn unbase32(s: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in s.chars() {
        // Crockford: I and L read as 1, O as 0; case doesn't matter.
        let c = match c.to_ascii_uppercase() {
            'I' | 'L' => '1',
            'O' => '0',
            c => c,
        };
        let v = CROCKFORD.iter().position(|&x| x as char == c).ok_or_else(|| bad(format!("`{c}` isn't in a pairing code")))? as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Ok(out)
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn dec(s: &str) -> Result<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let h = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()).ok_or_else(|| bad("bad escape"))?;
                out.push(h);
                i += 3;
            }
            b'%' => return Err(bad("bad escape")),
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).map_err(|_| bad("bad text"))
}

// ---- Calls -------------------------------------------------------------------------------

/// `POST /v1/pair` (no token yet).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairRequest {
    pub secret: String,
    /// What the desktop lists this phone as (`Pixel 9`).
    pub label: String,
}

/// The answer to a pairing: the long-lived token for this phone (only ever sent once).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairReply {
    pub device_id: String,
    pub token: String,
    pub desktop: Hello,
}

/// `GET /v1/hello`: the desktop and what it shares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub version: u32,
    pub label: String,
    pub roots: Vec<RootInfo>,
    /// Shell commands from a phone wait for a yes on the desktop.
    pub confirm_shell: bool,
    /// Direct addresses now (without `/p2p/`), so the phone keeps up with a changed LAN.
    #[serde(default)]
    pub direct: Vec<String>,
    /// Relay indexes the desktop is reachable through now.
    #[serde(default)]
    pub relays: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootInfo {
    /// The folder's name (`bragi`).
    pub name: String,
    /// Its full path on the desktop, for showing.
    pub path: String,
}

/// A path under one of the roots: `root` is its index in [`Hello::roots`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathRequest {
    pub root: usize,
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListReply {
    pub entries: Vec<Entry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadReply {
    /// `None`: the file doesn't exist.
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatherRequest {
    pub root: usize,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatherReply {
    pub files: Vec<ContextFile>,
    pub skipped: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireChange {
    pub path: String,
    /// What the file must hold now; `None`: it must not exist.
    pub before: Option<String>,
    pub after: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyRequest {
    pub root: usize,
    pub changes: Vec<WireChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoRequest {
    pub root: usize,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedReply {
    pub id: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectReply {
    /// The folder, relative to the root.
    pub path: String,
    /// What was set up (`Created README.md and .gitignore`), empty when it already had files.
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellRequest {
    pub root: usize,
    /// The folder to run in, relative to the root.
    #[serde(default)]
    pub cwd: String,
    pub command: String,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

/// One line of a shell call's streamed body (newline-delimited JSON).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ShellFrame {
    /// The desktop is asking its user first (sent again every few seconds while it waits).
    Waiting,
    /// Nothing new; keeps a quiet command's call from timing out.
    Alive,
    Out {
        text: String,
    },
    Err {
        text: String,
    },
    Exit {
        code: Option<i32>,
        timed_out: bool,
        stopped: bool,
        elapsed_ms: u64,
    },
    /// The desktop's user said no.
    Denied {
        message: String,
    },
}

/// The body of an error answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireError {
    pub kind: ErrorKind,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_links_round_trip() {
        let peer = weirdnet_net::Keypair::generate_ed25519().public().to_peer_id().to_string();
        let l = PairLink {
            peer: peer.clone(),
            secret: "00ff".into(),
            label: "Bragi on desk & co".into(),
            direct: vec!["/ip4/192.0.2.4/tcp/47474".into()],
            relays: vec![0],
            expires_ms: 123,
        };
        let url = l.to_url();
        assert!(url.starts_with("weirdnet-bragi://desktop?v=1&"));
        assert_eq!(PairLink::parse(&url).unwrap(), l);
        let addrs = l.addresses();
        assert_eq!(addrs[0], format!("/ip4/192.0.2.4/tcp/47474/p2p/{peer}"));
        assert!(is_circuit(&addrs[1]) && addrs[1].ends_with(&format!("/p2p-circuit/p2p/{peer}")));
        assert!(PairLink::parse("weirdnet-bragi://link?x").is_err());
        assert!(PairLink::parse(&url.replace("v=1", "v=9")).is_err());
    }

    #[test]
    fn short_codes_round_trip_and_forgive_typos() {
        let peer = weirdnet_net::Keypair::generate_ed25519().public().to_peer_id();
        let c = ShortCode { ip: Ipv4Addr::new(192, 0, 2, 7), port: 47474, secret: [1, 2, 3, 4, 5, 6], fingerprint: fingerprint(&peer) };
        let s = c.encode();
        assert_eq!(s.len(), 28 + 6);
        assert_eq!(s.split('-').count(), 7);
        assert_eq!(ShortCode::parse(&s).unwrap(), c);
        assert_eq!(ShortCode::parse(&s.to_lowercase().replace('-', " ")).unwrap(), c);
        assert!(ShortCode::parse("ABCD").is_err());
        assert!(ShortCode::parse(&s.replacen(&s[0..1], "U", 1)).is_err());
    }

    #[test]
    fn shell_frames_are_tagged() {
        let f = ShellFrame::Out { text: "hi\n".into() };
        assert_eq!(serde_json::to_string(&f).unwrap(), r#"{"t":"out","text":"hi\n"}"#);
    }
}
