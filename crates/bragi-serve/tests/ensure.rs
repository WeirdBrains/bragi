//! `ensure` against the real `bragi-serve` binary (with its demo node, so no network): it starts
//! one server once, reuses it, replaces a stale pid file, and leaves foreign listeners alone.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Barrier};

use bragi_serve::ensure::{self, EnsureError, Options, Outcome, Probe, Spawn};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn options(home: &Path, port: u16) -> Options {
    let mut o = Options::new(home.to_path_buf(), port);
    o.spawn = Some(Spawn { program: env!("CARGO_BIN_EXE_bragi-serve").into(), args: vec![] });
    o.extra_args = vec!["--demo".into()];
    o.idle_minutes = 5;
    o.timeout = std::time::Duration::from_secs(60);
    o
}

/// Stops a server this test started (by the pid it wrote).
fn stop(home: &Path) {
    if let Some((pid, _)) = ensure::read_pid_file(home) {
        #[cfg(unix)]
        let _ = std::process::Command::new("kill").arg(pid.to_string()).status();
        #[cfg(windows)]
        let _ = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).status();
    }
    for _ in 0..100 {
        if ensure::read_pid_file(home).is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

#[test]
fn starts_once_reuses_and_survives_its_caller() {
    let d = tempfile::tempdir().unwrap();
    let port = free_port();
    let o = options(d.path(), port);
    assert_eq!(ensure::probe(d.path(), port), Probe::Down);
    // Several callers at once: one server, one of them starts it.
    let gate = Arc::new(Barrier::new(4));
    let hs: Vec<_> = (0..4)
        .map(|_| {
            let (o, gate) = (o.clone(), gate.clone());
            std::thread::spawn(move || {
                gate.wait();
                ensure::ensure(&o)
            })
        })
        .collect();
    let outs: Vec<Outcome> = hs.into_iter().map(|h| h.join().unwrap().unwrap()).collect();
    let started = outs.iter().filter(|o| matches!(o, Outcome::Started { .. })).count();
    assert_eq!(started, 1, "{outs:?}");
    assert!(matches!(ensure::probe(d.path(), port), Probe::Ready { .. }));
    assert_eq!(ensure::read_pid_file(d.path()).map(|(_, p)| p), Some(port));
    assert!(ensure::pid_path(d.path()).exists());
    assert!(!ensure::lock_path(d.path()).exists(), "the start lock is released");
    // And again: reused, and it's the same process.
    let pid = ensure::read_pid_file(d.path()).unwrap().0;
    assert!(matches!(ensure::ensure(&o).unwrap(), Outcome::Reused { .. }));
    assert_eq!(ensure::read_pid_file(d.path()).unwrap().0, pid);
    stop(d.path());
    assert_eq!(ensure::probe(d.path(), port), Probe::Down);
}

#[test]
fn a_stale_pid_file_is_replaced() {
    let d = tempfile::tempdir().unwrap();
    let port = free_port();
    std::fs::create_dir_all(d.path().join("serve")).unwrap();
    std::fs::write(ensure::pid_path(d.path()), format!("999999\n{port}\n")).unwrap();
    // A start lock left by a dead starter is only taken over once it is old; a fresh one waits.
    let o = options(d.path(), port);
    let out = ensure::ensure(&o).unwrap();
    assert!(matches!(out, Outcome::Started { .. }));
    let (pid, p) = ensure::read_pid_file(d.path()).unwrap();
    assert_ne!(pid, 999_999);
    assert_eq!(p, port);
    stop(d.path());
}

#[test]
fn a_port_taken_by_something_else_is_left_alone() {
    let d = tempfile::tempdir().unwrap();
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let t = std::thread::spawn(move || {
        // Answers the probe with a page that isn't Bragi's.
        let (mut s, _) = l.accept().unwrap();
        let mut b = [0u8; 1024];
        let _ = s.read(&mut b);
        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        l
    });
    let e = ensure::ensure(&options(d.path(), port)).unwrap_err();
    assert_eq!(e, EnsureError::PortTaken(port));
    let _listener_still_ours = t.join().unwrap();
    assert!(!ensure::pid_path(d.path()).exists(), "nothing was started");
}

#[test]
fn a_server_for_another_home_is_not_taken_over() {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let port = free_port();
    assert!(matches!(ensure::ensure(&options(a.path(), port)).unwrap(), Outcome::Started { .. }));
    // b has a token of its own.
    bragi_serve::load_or_create_token(b.path()).unwrap();
    let e = ensure::ensure(&options(b.path(), port)).unwrap_err();
    assert_eq!(e, EnsureError::OtherToken(port));
    assert!(matches!(ensure::probe(a.path(), port), Probe::Ready { .. }), "the first server is still running");
    stop(a.path());
}

#[test]
fn no_binary_is_reported() {
    let d = tempfile::tempdir().unwrap();
    // A test running beside this one may take the port free_port released; try a fresh one then.
    let mut e = None;
    for _ in 0..5 {
        let mut o = options(d.path(), free_port());
        o.spawn = Some(Spawn { program: d.path().join("no-such-bragi"), args: vec![] });
        match ensure::ensure(&o).unwrap_err() {
            EnsureError::PortTaken(_) => continue,
            other => {
                e = Some(other);
                break;
            }
        }
    }
    let e = e.expect("every port tried was taken");
    assert!(matches!(&e, EnsureError::Failed(m) if m.contains("couldn't start")), "{e}");
}
