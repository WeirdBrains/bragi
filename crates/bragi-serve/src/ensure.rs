//! Making sure `bragi serve` is running: shared by the terminal app, the editor extension's
//! behaviour (which does the same in TypeScript) and anything else that needs the local API.
//!
//! [`ensure`] checks for a healthy server on the port (health, then the token), reuses it, or
//! starts `bragi serve` detached in the background and waits until it answers. Only one starter
//! runs at a time (a lock file under `<home>/serve/`), a server that was already there is never
//! touched, and nothing here ever kills a process. A server started this way shuts itself down
//! after [`IDLE_MINUTES`] without a client (`--idle-timeout`), so it doesn't linger.
//!
//! Files under `<home>/serve/`: `token` (the API token), `server.pid` (written by the running
//! server: its pid and port; removed when it stops), `start.lock` (held while one process starts a
//! server) and `server.log` (what a started server prints).

use std::fmt;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use crate::token_path;

/// A server that `ensure` started stops after this long with no client.
pub const IDLE_MINUTES: u64 = 30;
/// How long `ensure` waits for a started server to answer.
pub const START_TIMEOUT: Duration = Duration::from_secs(25);
/// A start lock older than this is taken to be left behind by a crashed starter.
const LOCK_STALE: Duration = Duration::from_secs(60);

pub fn pid_path(home: &Path) -> PathBuf {
    home.join("serve").join("server.pid")
}

pub fn lock_path(home: &Path) -> PathBuf {
    home.join("serve").join("start.lock")
}

pub fn log_path(home: &Path) -> PathBuf {
    home.join("serve").join("server.log")
}

/// What answers on the port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// A Bragi server that accepts this home's token.
    Ready { version: String },
    /// A Bragi server that refuses this home's token (another Bragi home), or there is no token yet.
    OtherToken { version: String },
    /// Something else is listening on the port.
    NotBragi,
    /// Nothing is listening.
    Down,
}

/// What `ensure` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A server was already there; it was left as it was.
    Reused { version: String },
    /// This call started the server.
    Started { version: String },
}

impl Outcome {
    pub fn version(&self) -> &str {
        match self {
            Outcome::Reused { version } | Outcome::Started { version } => version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnsureError {
    /// No `bragi` or `bragi-serve` was found to start.
    NoBinary(String),
    /// Something that isn't Bragi's API is on the port.
    PortTaken(u16),
    /// A Bragi server on the port doesn't take this home's token.
    OtherToken(u16),
    /// The server didn't answer in time or exited.
    Failed(String),
}

impl fmt::Display for EnsureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnsureError::NoBinary(m) | EnsureError::Failed(m) => f.write_str(m),
            EnsureError::PortTaken(p) => write!(f, "port {p} is in use by something that isn't Bragi's API (use --port, or free it)"),
            EnsureError::OtherToken(p) => write!(f, "a Bragi server on port {p} doesn't accept this Bragi home's token (it uses another home)"),
        }
    }
}

impl std::error::Error for EnsureError {}

/// The program and arguments that start a server (before `--home` and the rest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spawn {
    pub program: PathBuf,
    pub args: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub home: PathBuf,
    pub port: u16,
    /// Stop a server this call starts after this many idle minutes (0: never).
    pub idle_minutes: u64,
    pub timeout: Duration,
    /// Start this instead of looking for `bragi` (`bragi.path` in the extension).
    pub spawn: Option<Spawn>,
    /// More arguments for the started server (`--demo` in tests).
    pub extra_args: Vec<String>,
}

impl Options {
    pub fn new(home: PathBuf, port: u16) -> Self {
        Self { home, port, idle_minutes: IDLE_MINUTES, timeout: START_TIMEOUT, spawn: None, extra_args: Vec::new() }
    }
}

// ---- probing ----------------------------------------------------------------------------------

/// One HTTP/1.1 request to the loopback port: `(status, body)`. Plain std, no client library.
fn http_get(port: u16, path: &str, token: Option<&str>) -> std::io::Result<(u16, String)> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(800))?;
    s.set_read_timeout(Some(Duration::from_secs(3)))?;
    s.set_write_timeout(Some(Duration::from_secs(3)))?;
    let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    write!(s, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{auth}Connection: close\r\n\r\n")?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if raw.len() > 256 * 1024 {
                    break;
                }
            }
            Err(e) if raw.is_empty() => return Err(e),
            Err(_) => break,
        }
    }
    let text = String::from_utf8_lossy(&raw);
    let status = text
        .strip_prefix("HTTP/1.")
        .and_then(|r| r.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not an HTTP answer"))?;
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    Ok((status, body))
}

fn health_version(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    if v.get("ok")? != &serde_json::Value::Bool(true) {
        return None;
    }
    Some(v.get("version")?.as_str()?.to_string())
}

/// Looks at the port: is a Bragi server there, and does it take this home's token?
pub fn probe(home: &Path, port: u16) -> Probe {
    let (status, body) = match http_get(port, "/v1/health", None) {
        Ok(r) => r,
        Err(e) if matches!(e.kind(), std::io::ErrorKind::ConnectionRefused) => return Probe::Down,
        // Connected (or timed out) but not an HTTP answer: not ours. A connect timeout on
        // loopback is as good as nothing listening.
        Err(e) if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::HostUnreachable | std::io::ErrorKind::NetworkUnreachable) => {
            return Probe::Down;
        }
        Err(_) => return Probe::NotBragi,
    };
    let Some(version) = (status == 200).then(|| health_version(&body)).flatten() else { return Probe::NotBragi };
    let Ok(token) = std::fs::read_to_string(token_path(home)) else { return Probe::OtherToken { version } };
    match http_get(port, "/v1/state", Some(token.trim())) {
        Ok((200, _)) => Probe::Ready { version },
        _ => Probe::OtherToken { version },
    }
}

// ---- finding the binary -----------------------------------------------------------------------

fn exe_names(base: &str) -> Vec<String> {
    if cfg!(windows) { vec![format!("{base}.exe"), format!("{base}.cmd")] } else { vec![base.to_string()] }
}

fn on_path(base: &str, path: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let path = path?;
    for dir in std::env::split_paths(&path) {
        for n in exe_names(base) {
            let p = dir.join(&n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// What starts a server: `$BRAGI_BIN` (a `bragi` or `bragi-serve`), the running program if it is
/// `bragi`, a `bragi-serve` or `bragi` next to it, then `bragi` and `bragi-serve` on the `PATH`.
pub fn find_spawn() -> Result<Spawn, EnsureError> {
    let for_name = |p: PathBuf| {
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
        Spawn { args: if stem == "bragi-serve" { vec![] } else { vec!["serve".into()] }, program: p }
    };
    if let Some(b) = std::env::var_os("BRAGI_BIN").filter(|b| !b.is_empty()) {
        let p = PathBuf::from(b);
        return if p.is_file() { Ok(for_name(p)) } else { Err(EnsureError::NoBinary(format!("BRAGI_BIN ({}) isn't a file", p.display()))) };
    }
    if let Ok(exe) = std::env::current_exe() {
        let stem = exe.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if stem == "bragi" || stem == "bragi-serve" {
            return Ok(for_name(exe));
        }
        if let Some(dir) = exe.parent() {
            for base in ["bragi-serve", "bragi"] {
                for n in exe_names(base) {
                    let p = dir.join(n);
                    if p.is_file() {
                        return Ok(for_name(p));
                    }
                }
            }
        }
    }
    let path = std::env::var_os("PATH");
    for base in ["bragi", "bragi-serve"] {
        if let Some(p) = on_path(base, path.clone()) {
            return Ok(for_name(p));
        }
    }
    Err(EnsureError::NoBinary("couldn't find `bragi` to start the server: put it on the PATH, or run `bragi serve` yourself".into()))
}

// ---- starting ---------------------------------------------------------------------------------

/// Holds the start lock; removes it when dropped.
struct StartLock(PathBuf);

impl StartLock {
    fn try_take(home: &Path) -> std::io::Result<Option<Self>> {
        let path = lock_path(home);
        std::fs::create_dir_all(path.parent().unwrap_or(home))?;
        for _ in 0..2 {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    let _ = write!(f, "{}", std::process::id());
                    return Ok(Some(Self(path)));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let age = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok());
                    if age.is_some_and(|a| a > LOCK_STALE) {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    return Ok(None);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }
}

impl Drop for StartLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Starts the program detached: its own process group (Unix) or without a console (Windows), no
/// terminal input or output, so it outlives the caller and Ctrl-C in the caller's terminal.
fn spawn_detached(spawn: &Spawn, o: &Options) -> Result<std::process::Child, EnsureError> {
    let mut cmd = Command::new(&spawn.program);
    cmd.args(&spawn.args).arg("--home").arg(&o.home).arg("--port").arg(o.port.to_string());
    if o.idle_minutes > 0 {
        cmd.arg("--idle-timeout").arg(o.idle_minutes.to_string());
    }
    cmd.args(&o.extra_args);
    let log = std::fs::create_dir_all(o.home.join("serve"))
        .ok()
        .and_then(|()| std::fs::OpenOptions::new().create(true).append(true).open(log_path(&o.home)).ok());
    cmd.stdin(Stdio::null());
    match log.and_then(|l| l.try_clone().ok().map(|c| (l, c))) {
        Some((out, err)) => {
            cmd.stdout(out).stderr(err);
        }
        None => {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
        cmd.creation_flags(0x0000_0008 | 0x0000_0200 | 0x0800_0000);
    }
    cmd.spawn().map_err(|e| EnsureError::Failed(format!("couldn't start {}: {e}", spawn.program.display())))
}

fn wait_ready(home: &Path, port: u16, until: Instant, mut child: Option<&mut std::process::Child>) -> Result<Probe, EnsureError> {
    loop {
        match probe(home, port) {
            p @ (Probe::Ready { .. } | Probe::NotBragi) => return Ok(p),
            // Our own child can show this while its token file isn't there yet: keep waiting.
            p @ Probe::OtherToken { .. } if child.is_none() => return Ok(p),
            _ => {}
        }
        if let Some(c) = child.as_deref_mut()
            && let Ok(Some(status)) = c.try_wait()
        {
            return Err(EnsureError::Failed(format!("the server exited ({status}); see {}", log_path(home).display())));
        }
        if Instant::now() >= until {
            return Err(EnsureError::Failed(format!("the server didn't answer in time; see {}", log_path(home).display())));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn settle(home: &Path, port: u16, p: Probe, started: bool) -> Result<Outcome, EnsureError> {
    match p {
        Probe::Ready { version } => Ok(if started { Outcome::Started { version } } else { Outcome::Reused { version } }),
        Probe::OtherToken { .. } => Err(EnsureError::OtherToken(port)),
        Probe::NotBragi => Err(EnsureError::PortTaken(port)),
        Probe::Down => Err(EnsureError::Failed(format!("no server on port {port}; see {}", log_path(home).display()))),
    }
}

/// Makes sure a healthy server is on `o.port` for `o.home`'s token: reuses one, or starts one.
/// Blocks for up to `o.timeout`; run it off the UI thread.
pub fn ensure(o: &Options) -> Result<Outcome, EnsureError> {
    let until = Instant::now() + o.timeout;
    match probe(&o.home, o.port) {
        Probe::Ready { version } => return Ok(Outcome::Reused { version }),
        Probe::OtherToken { .. } => return Err(EnsureError::OtherToken(o.port)),
        Probe::NotBragi => return Err(EnsureError::PortTaken(o.port)),
        Probe::Down => {}
    }
    let spawn = match &o.spawn {
        Some(s) => s.clone(),
        None => find_spawn()?,
    };
    loop {
        match StartLock::try_take(&o.home).map_err(|e| EnsureError::Failed(format!("{}: {e}", lock_path(&o.home).display())))? {
            Some(_lock) => {
                // Someone may have finished starting one while we waited for the lock.
                match probe(&o.home, o.port) {
                    Probe::Down => {}
                    p => return settle(&o.home, o.port, p, false),
                }
                // A pid file with nothing on the port is a server that died: forget it.
                let _ = std::fs::remove_file(pid_path(&o.home));
                let mut child = spawn_detached(&spawn, o)?;
                let p = wait_ready(&o.home, o.port, until, Some(&mut child))?;
                // `child` is dropped without waiting: the server keeps running.
                return settle(&o.home, o.port, p, true);
            }
            None => {
                // Another process is starting one: wait for it.
                match probe(&o.home, o.port) {
                    Probe::Down => {}
                    p => return settle(&o.home, o.port, p, false),
                }
                if Instant::now() >= until {
                    return Err(EnsureError::Failed("another process is starting the server and it isn't answering".into()));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// The running server's pid and port from `server.pid`, if the file is there.
pub fn read_pid_file(home: &Path) -> Option<(u32, u16)> {
    let s = std::fs::read_to_string(pid_path(home)).ok()?;
    let mut l = s.lines();
    Some((l.next()?.trim().parse().ok()?, l.next()?.trim().parse().ok()?))
}

/// Written by a running server: its pid and port.
pub fn write_pid_file(home: &Path, port: u16) -> std::io::Result<()> {
    let p = pid_path(home);
    std::fs::create_dir_all(p.parent().unwrap_or(home))?;
    std::fs::write(p, format!("{}\n{port}\n", std::process::id()))
}

/// Removes `server.pid` if it is this process's.
pub fn remove_pid_file(home: &Path) {
    if read_pid_file(home).is_some_and(|(pid, _)| pid == std::process::id()) {
        let _ = std::fs::remove_file(pid_path(home));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_bodies_are_checked() {
        assert_eq!(health_version(r#"{"ok":true,"version":"1.2.3"}"#).as_deref(), Some("1.2.3"));
        assert_eq!(health_version(r#"{"ok":false,"version":"1"}"#), None);
        assert_eq!(health_version("<html>"), None);
    }

    #[test]
    fn a_free_port_is_down_and_a_foreign_listener_is_not_bragi() {
        let d = tempfile::tempdir().unwrap();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let t = std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut b = [0u8; 512];
                let _ = s.read(&mut b);
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi");
            }
        });
        assert_eq!(probe(d.path(), port), Probe::NotBragi);
        t.join().unwrap();
        // Nothing listening: a port just let go of. Tests run in parallel and another one may
        // take that port straight away, so a few fresh ports get a chance before it fails.
        let seen: Vec<Probe> = (0..5)
            .map(|_| {
                let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
                probe(d.path(), port)
            })
            .take_while(|p| *p != Probe::Down)
            .collect();
        assert!(seen.len() < 5, "no free port probed as down: {seen:?}");
    }

    #[test]
    fn the_pid_file_is_this_processes_to_remove() {
        let d = tempfile::tempdir().unwrap();
        write_pid_file(d.path(), 4877).unwrap();
        assert_eq!(read_pid_file(d.path()), Some((std::process::id(), 4877)));
        remove_pid_file(d.path());
        assert!(read_pid_file(d.path()).is_none());
        std::fs::write(pid_path(d.path()), "1\n4877\n").unwrap();
        remove_pid_file(d.path());
        assert!(pid_path(d.path()).exists(), "another process's pid file stays");
    }

    #[test]
    fn a_stale_lock_is_taken_over_and_a_live_one_is_not() {
        let d = tempfile::tempdir().unwrap();
        let a = StartLock::try_take(d.path()).unwrap().unwrap();
        assert!(StartLock::try_take(d.path()).unwrap().is_none());
        drop(a);
        let b = StartLock::try_take(d.path()).unwrap();
        assert!(b.is_some());
    }

    #[test]
    fn a_missing_binary_is_said_plainly() {
        let d = tempfile::tempdir().unwrap();
        let mut o = Options::new(d.path().into(), 1);
        o.spawn = Some(Spawn { program: d.path().join("nope"), args: vec![] });
        // Port 1 is never listening.
        let e = ensure(&o).unwrap_err();
        assert!(matches!(e, EnsureError::Failed(m) if m.contains("couldn't start")));
    }
}
