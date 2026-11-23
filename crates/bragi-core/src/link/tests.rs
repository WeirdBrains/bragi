//! The link end to end over the in-memory transport: pairing, the calls, the shell, and every
//! way a call is turned away.

use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::error::ErrorKind;

struct Fixture {
    _d: tempfile::TempDir,
    proj: std::path::PathBuf,
    desk_dir: std::path::PathBuf,
    server: LinkServer,
    phone: LinkClient,
}

fn fixture(confirm: Option<Confirm>, confirm_shell: bool) -> Fixture {
    let d = tempfile::tempdir().unwrap();
    let proj = d.path().join("proj");
    std::fs::create_dir_all(proj.join("src")).unwrap();
    std::fs::write(proj.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(d.path().join("outside.txt"), "private").unwrap();
    let desk_dir = d.path().join("desk/linker");
    let mut s = Settings::default();
    s.add_root(&proj).unwrap();
    s.confirm_shell = confirm_shell;
    s.save(&desk_dir).unwrap();
    let opts = ServerOptions { memory: true, ..ServerOptions::new(&desk_dir, "Bragi on test-desk") };
    let server = LinkServer::start(opts, confirm).unwrap();
    let phone = LinkClient::open(d.path().join("phone/linker"), true);
    Fixture { proj: proj.canonicalize().unwrap(), _d: d, desk_dir, server, phone }
}

/// A pairing link that points at the server's memory address (no LAN in tests).
fn memory_link(f: &Fixture) -> String {
    let mut o = f.server.offer(Duration::from_secs(60));
    o.link.direct = f.server.direct();
    assert!(!o.link.direct.is_empty());
    o.link.to_url()
}

async fn paired() -> (Fixture, String) {
    let f = fixture(None, false);
    let url = memory_link(&f);
    let v = f.phone.pair(&url, "Test phone").await.unwrap();
    assert_eq!(v.label, "Bragi on test-desk");
    assert_eq!(v.roots.len(), 1);
    (f, v.id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairs_then_reads_writes_and_undoes() {
    let (f, id) = paired().await;
    assert_eq!(server::devices(&f.desk_dir).len(), 1);
    // The token is stored hashed on the desktop, never as is.
    let devices = std::fs::read_to_string(f.desk_dir.join("devices.json")).unwrap();
    let client = std::fs::read_to_string(f.phone.dir().join("client.json")).unwrap();
    let token = serde_json::from_str::<serde_json::Value>(&client).unwrap()["desktops"][0]["token"].as_str().unwrap().to_string();
    assert!(!devices.contains(&token));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(f.desk_dir.join("devices.json")).unwrap().permissions().mode() & 0o077, 0);
        assert_eq!(std::fs::metadata(f.phone.dir().join("client.json")).unwrap().permissions().mode() & 0o077, 0);
    }

    let h = f.phone.hello(&id).await.unwrap();
    assert_eq!(h.roots[0].name, "proj");
    let names: Vec<String> = f.phone.list(&id, 0, "").await.unwrap().into_iter().map(|e| e.name).collect();
    assert_eq!(names, ["src"]);
    assert_eq!(f.phone.read(&id, 0, "src/main.rs").await.unwrap().as_deref(), Some("fn main() {}\n"));
    let g = f.phone.gather(&id, 0, &["src".into()]).await.unwrap();
    assert_eq!(g.files[0].path, "src/main.rs");
    let a = f
        .phone
        .apply(&id, 0, vec![WireChange { path: "src/main.rs".into(), before: Some("fn main() {}\n".into()), after: "fn main() { run() }\n".into() }])
        .await
        .unwrap();
    assert_eq!(a.files, ["src/main.rs"]);
    assert_eq!(std::fs::read_to_string(f.proj.join("src/main.rs")).unwrap(), "fn main() { run() }\n");
    f.phone.undo(&id, 0, false).await.unwrap();
    assert_eq!(std::fs::read_to_string(f.proj.join("src/main.rs")).unwrap(), "fn main() {}\n");
    let p = f.phone.project(&id, 0, "newapp").await.unwrap();
    assert!(f.proj.join("newapp/README.md").exists(), "{p:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn paths_outside_the_roots_are_refused() {
    let (f, id) = paired().await;
    for bad in ["../outside.txt", "/etc/passwd", "src/../../outside.txt"] {
        let e = f.phone.read(&id, 0, bad).await.unwrap_err();
        assert_eq!(e.kind, ErrorKind::Refused, "{bad}: {e:?}");
    }
    let e = f.phone.apply(&id, 0, vec![WireChange { path: "../evil.txt".into(), before: None, after: "x".into() }]).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Refused);
    assert!(!f.proj.parent().unwrap().join("evil.txt").exists());
    assert_eq!(f.phone.list(&id, 7, "").await.unwrap_err().kind, ErrorKind::NotFound);
    let e =
        f.phone.shell(&id, ShellRequest { root: 0, cwd: "..".into(), command: "true".into(), timeout_secs: None }, &mut |_| {}).await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Refused);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shell_streams_and_stops_when_dropped() {
    let (f, id) = paired().await;
    let mut frames = Vec::new();
    let end = f
        .phone
        .shell(&id, ShellRequest { root: 0, cwd: "src".into(), command: "ls; echo bad >&2; exit 2".into(), timeout_secs: None }, &mut |fr| {
            frames.push(fr)
        })
        .await
        .unwrap();
    assert!(matches!(end, ShellFrame::Exit { code: Some(2), .. }), "{end:?}");
    assert!(frames.iter().any(|fr| matches!(fr, ShellFrame::Out { text } if text.contains("main.rs"))));
    assert!(frames.iter().any(|fr| matches!(fr, ShellFrame::Err { text } if text.contains("bad"))));

    // Dropping the call (the phone's stop) kills the command: the marker is never written.
    let marker = f.proj.join("marker");
    let cmd = format!("sleep 5; touch {}", marker.display());
    let mut sink = |_| {};
    let run = f.phone.shell(&id, ShellRequest { root: 0, cwd: String::new(), command: cmd, timeout_secs: None }, &mut sink);
    let _ = tokio::time::timeout(Duration::from_millis(600), run).await;
    // The stop reaches the desktop with the next keep-alive frame (about a second).
    tokio::time::sleep(Duration::from_secs(7)).await;
    assert!(!marker.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codes_are_one_use_and_expire() {
    let f = fixture(None, false);
    let url = memory_link(&f);
    f.phone.pair(&url, "one").await.unwrap();
    // Used: a second phone with the same code is turned away.
    let other = LinkClient::open(f._d.path().join("phone2/linker"), true);
    let e = other.pair(&url, "two").await.unwrap_err();
    assert_eq!(e.kind, ErrorKind::Pairing);
    // Wrong secrets: after a few the code is cancelled.
    let url = memory_link(&f);
    let mut link = PairLink::parse(&url).unwrap();
    link.secret = "00".into();
    for _ in 0..server::MAX_TRIES {
        assert_eq!(other.pair(&link.to_url(), "two").await.unwrap_err().kind, ErrorKind::Pairing);
    }
    assert!(!f.server.offering());
    assert!(other.pair(&url, "two").await.is_err());
    // Expired.
    let mut o = f.server.offer(Duration::from_millis(1));
    o.link.direct = f.server.direct();
    o.link.expires_ms = crate::history::now_ms() + 60_000;
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(other.pair(&o.link.to_url(), "two").await.unwrap_err().kind, ErrorKind::Pairing);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_phones_and_stolen_tokens_are_refused() {
    let (f, id) = paired().await;
    // Another key presenting the first phone's token: the peer id doesn't match.
    let thief_dir = f._d.path().join("thief/linker");
    std::fs::create_dir_all(&thief_dir).unwrap();
    std::fs::copy(f.phone.dir().join("client.json"), thief_dir.join("client.json")).unwrap();
    let thief = LinkClient::open(&thief_dir, true);
    assert_eq!(thief.hello(&id).await.unwrap_err().kind, ErrorKind::NotPaired);
    // Revoked on the desktop (as `bragi-tui linker revoke` does, while the server runs).
    let gone = server::revoke(&f.desk_dir, "Test phone").unwrap();
    assert_eq!(gone.len(), 1);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(f.phone.hello(&id).await.unwrap_err().kind, ErrorKind::NotPaired);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_phone_can_unpair_itself() {
    let (f, id) = paired().await;
    f.phone.forget(&id).await.unwrap();
    assert!(f.phone.desktops().is_empty());
    assert!(server::devices(&f.desk_dir).is_empty());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shell_waits_for_the_desktop_when_asked_to() {
    let asked = Arc::new(std::sync::Mutex::new(Vec::<ShellAsk>::new()));
    let a = asked.clone();
    let confirm: Confirm = Arc::new(move |ask: ShellAsk| {
        let yes = ask.command.starts_with("echo");
        a.lock().unwrap().push(ask);
        Box::pin(async move { yes })
    });
    let f = fixture(Some(confirm), true);
    let url = memory_link(&f);
    let id = f.phone.pair(&url, "p").await.unwrap().id;
    let mut frames = Vec::new();
    let end = f
        .phone
        .shell(&id, ShellRequest { root: 0, cwd: String::new(), command: "echo ok".into(), timeout_secs: None }, &mut |fr| frames.push(fr))
        .await
        .unwrap();
    assert!(matches!(end, ShellFrame::Exit { code: Some(0), .. }));
    assert_eq!(frames.first(), Some(&ShellFrame::Waiting));
    let end = f
        .phone
        .shell(&id, ShellRequest { root: 0, cwd: String::new(), command: "rm -rf src".into(), timeout_secs: None }, &mut |_| {})
        .await
        .unwrap();
    assert!(matches!(end, ShellFrame::Denied { .. }));
    assert!(f.proj.join("src").exists());
    assert_eq!(asked.lock().unwrap().len(), 2);
    assert_eq!(asked.lock().unwrap()[0].device, "p");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn routes_order_the_addresses() {
    let f = fixture(None, false);
    let peer = f.server.peer_id();
    let link = PairLink {
        peer: peer.clone(),
        secret: "x".into(),
        label: "d".into(),
        direct: vec!["/ip4/192.0.2.1/tcp/1".into()],
        relays: vec![0],
        expires_ms: 0,
    };
    let all = link.addresses();
    assert_eq!(all.len(), 2);
    f.phone.set_route(Route::Weirdnet).unwrap();
    assert_eq!(f.phone.route(), Route::Weirdnet);
    assert_eq!(LinkClient::open(f.phone.dir(), true).route(), Route::Weirdnet);
    assert_eq!(Route::parse("relay"), Some(Route::Weirdnet));
}

/// The real network: a desktop on this machine's LAN addresses and the Weirdnet relay, a phone
/// reaching it once directly and once through the relay only. Needs the internet, so it is
/// ignored by default: `cargo test -p bragi-core link::tests::live -- --ignored`.
#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_direct_and_weirdnet_routes() {
    let d = tempfile::tempdir().unwrap();
    let proj = d.path().join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join("hello.txt"), "hi\n").unwrap();
    let desk = d.path().join("desk/linker");
    let mut s = Settings::default();
    s.add_root(&proj).unwrap();
    s.save(&desk).unwrap();
    let mut opts = ServerOptions::new(&desk, "live desk");
    opts.port = 0;
    let server = LinkServer::start(opts, None).unwrap();
    for _ in 0..100 {
        if server.on_weirdnet() && !server.direct().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(server.on_weirdnet(), "no relay reservation");
    let o = server.offer(Duration::from_secs(60));
    let phone = LinkClient::open(d.path().join("phone/linker"), false);
    phone.set_route(Route::Direct).unwrap();
    let id = phone.pair(&o.url, "live phone").await.unwrap().id;
    assert_eq!(phone.read(&id, 0, "hello.txt").await.unwrap().as_deref(), Some("hi\n"));
    phone.set_route(Route::Weirdnet).unwrap();
    // Weirdnet only: drop the direct addresses so only the relay circuit is left.
    {
        let mut f: serde_json::Value = serde_json::from_slice(&std::fs::read(phone.dir().join("client.json")).unwrap()).unwrap();
        f["desktops"][0]["direct"] = serde_json::json!([]);
        std::fs::write(phone.dir().join("client.json"), serde_json::to_vec(&f).unwrap()).unwrap();
    }
    let phone = LinkClient::open(d.path().join("phone/linker"), false);
    let mut out = String::new();
    let end = phone
        .shell(&id, ShellRequest { root: 0, cwd: String::new(), command: "cat hello.txt".into(), timeout_secs: None }, &mut |f| {
            if let ShellFrame::Out { text } = f {
                out.push_str(&text);
            }
        })
        .await
        .unwrap();
    assert!(matches!(end, ShellFrame::Exit { code: Some(0), .. }));
    assert_eq!(out, "hi\n");
    // The typed code: dialed by address, the key checked against the code's fingerprint.
    let o = server.offer(Duration::from_secs(60));
    let short = o.short.expect("a LAN IPv4 address for the typed code");
    let typed = LinkClient::open(d.path().join("phone2/linker"), false);
    let v = typed.pair(&short, "typed phone").await.unwrap();
    assert_eq!(v.id, server.peer_id());
    assert_eq!(server::devices(&desk).len(), 2);
}
