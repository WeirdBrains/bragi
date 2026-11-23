//! Running a shell command the user typed (`!cmd` or `/shell cmd` in the terminal app) in the
//! project folder, and collecting what it printed. Only the user starts these: no model or tool
//! call reaches [`run`]. The command runs under `sh -c` (`cmd /C` on Windows) with no input, a
//! time limit, and a stop flag; output past [`MAX_OUTPUT`] keeps its end, where errors usually are.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::error::{CoreError, ErrorKind, Result};

/// The most of each stream kept, in bytes (the last part is kept).
pub const MAX_OUTPUT: usize = 64 * 1024;

/// How long a command may run unless the shell says otherwise.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// How a command ended, with what it printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOutput {
    pub command: String,
    pub cwd: PathBuf,
    pub stdout: String,
    pub stderr: String,
    /// The exit code; `None` when it was killed (time limit, stop, or a signal).
    pub code: Option<i32>,
    pub timed_out: bool,
    pub stopped: bool,
    /// Some output was cut (only its end is kept).
    pub truncated: bool,
    pub elapsed_ms: u64,
}

impl ShellOutput {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    /// `exit 0 · 0.3s`, `exit 2 · 1.2s`, `stopped · 4.0s`, `timed out after 600s`.
    pub fn status(&self) -> String {
        let secs = format!("{:.1}s", self.elapsed_ms as f64 / 1000.0);
        match (self.code, self.timed_out, self.stopped) {
            (_, true, _) => format!("timed out after {secs}"),
            (_, _, true) => format!("stopped · {secs}"),
            (Some(c), ..) => format!("exit {c} · {secs}"),
            (None, ..) => format!("killed by a signal · {secs}"),
        }
    }

    /// The whole run as plain text: `$ cmd`, stdout, stderr, the status line.
    pub fn transcript(&self) -> String {
        let mut s = format!("$ {}\n", self.command);
        for part in [&self.stdout, &self.stderr] {
            if !part.is_empty() {
                s.push_str(part);
                if !part.ends_with('\n') {
                    s.push('\n');
                }
            }
        }
        if self.truncated {
            s.push_str(&format!("[output cut to its last {} KiB per stream]\n", MAX_OUTPUT / 1024));
        }
        s.push_str(&format!("[{}]\n", self.status()));
        s
    }
}

/// A command line as typed after `!` or `/shell`: `None` when it is empty.
pub fn parse(line: &str) -> Option<&str> {
    let t = line.trim();
    let rest = match t.strip_prefix('!') {
        Some(r) => r,
        None => {
            let r = t.strip_prefix("/shell").or_else(|| t.strip_prefix("/sh"))?;
            // `/shellfish` is not `/shell fish`.
            if !(r.is_empty() || r.starts_with(char::is_whitespace)) {
                return None;
            }
            r
        }
    };
    let rest = rest.trim();
    (!rest.is_empty()).then_some(rest)
}

/// Whether `line` is a shell line at all (`!…`, `/shell…`, `/sh…`), even an empty one.
pub fn is_shell_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('!') || t == "/shell" || t == "/sh" || t.starts_with("/shell ") || t.starts_with("/sh ")
}

/// Runs `command` in `cwd` and waits for it, up to `timeout` or until `stop` is set.
pub fn run(command: &str, cwd: &Path, timeout: Duration, stop: &AtomicBool) -> Result<ShellOutput> {
    let started = Instant::now();
    let meta = std::fs::metadata(cwd).map_err(|e| CoreError::new(ErrorKind::Storage, format!("{}: {e}", cwd.display())))?;
    if !meta.is_dir() {
        return Err(CoreError::new(ErrorKind::BadRequest, format!("{} isn't a folder", cwd.display())));
    }
    let mut cmd = command_for(command, cwd);
    let mut child = cmd.spawn().map_err(|e| CoreError::new(ErrorKind::Other, format!("couldn't start the shell: {e}")))?;
    let out = reader(child.stdout.take());
    let err = reader(child.stderr.take());
    let (mut timed_out, mut stopped) = (false, false);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {}
            Err(_) => break None,
        }
        if started.elapsed() >= timeout {
            timed_out = true;
        } else if stop.load(Ordering::Relaxed) {
            stopped = true;
        }
        if timed_out || stopped {
            kill(&mut child);
            break child.wait().ok();
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let (stdout, t1) = out.join().unwrap_or_default();
    let (stderr, t2) = err.join().unwrap_or_default();
    let code = if timed_out || stopped { None } else { status.and_then(|s| s.code()) };
    Ok(ShellOutput {
        command: command.to_string(),
        cwd: cwd.to_path_buf(),
        stdout,
        stderr,
        code,
        timed_out,
        stopped,
        truncated: t1 || t2,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// How a streamed run ended ([`run_streaming`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ended {
    pub code: Option<i32>,
    pub timed_out: bool,
    pub stopped: bool,
    pub elapsed_ms: u64,
}

/// Like [`run`], but hands each piece of output to `on` as it arrives (`true` for stderr) and
/// keeps none of it. `stop` is asked every few milliseconds; when it says so the command and
/// everything it started are killed. Used by the desktop link, which streams to a phone.
pub fn run_streaming(command: &str, cwd: &Path, timeout: Duration, stop: &dyn Fn() -> bool, on: &mut dyn FnMut(bool, String)) -> Result<Ended> {
    let started = Instant::now();
    let meta = std::fs::metadata(cwd).map_err(|e| CoreError::new(ErrorKind::Storage, format!("{}: {e}", cwd.display())))?;
    if !meta.is_dir() {
        return Err(CoreError::new(ErrorKind::BadRequest, format!("{} isn't a folder", cwd.display())));
    }
    let mut child = command_for(command, cwd).spawn().map_err(|e| CoreError::new(ErrorKind::Other, format!("couldn't start the shell: {e}")))?;
    let (tx, rx) = std::sync::mpsc::channel::<(bool, String)>();
    let readers = [streamer(child.stdout.take(), false, tx.clone()), streamer(child.stderr.take(), true, tx)];
    let (mut timed_out, mut stopped) = (false, false);
    let status = loop {
        while let Ok((err, text)) = rx.try_recv() {
            on(err, text);
        }
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {}
            Err(_) => break None,
        }
        if started.elapsed() >= timeout {
            timed_out = true;
        } else if stop() {
            stopped = true;
        }
        if timed_out || stopped {
            kill(&mut child);
            break child.wait().ok();
        }
        if let Ok((err, text)) = rx.recv_timeout(Duration::from_millis(20)) {
            on(err, text);
        }
    };
    for r in readers {
        let _ = r.join();
    }
    while let Ok((err, text)) = rx.try_recv() {
        on(err, text);
    }
    let code = if timed_out || stopped { None } else { status.and_then(|s| s.code()) };
    Ok(Ended { code, timed_out, stopped, elapsed_ms: started.elapsed().as_millis() as u64 })
}

/// Sends a stream's text on as it comes, never splitting a UTF-8 character.
fn streamer<R: Read + Send + 'static>(r: Option<R>, err: bool, tx: std::sync::mpsc::Sender<(bool, String)>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let Some(mut r) = r else { return };
        let mut pending: Vec<u8> = Vec::new();
        let mut buf = [0u8; 8192];
        while let Ok(n) = r.read(&mut buf) {
            if n == 0 {
                break;
            }
            pending.extend_from_slice(&buf[..n]);
            let upto = match std::str::from_utf8(&pending) {
                Ok(_) => pending.len(),
                Err(e) if e.error_len().is_none() => e.valid_up_to(),
                Err(_) => pending.len(),
            };
            if upto > 0 {
                let rest = pending.split_off(upto);
                let text = String::from_utf8_lossy(&pending).into_owned();
                pending = rest;
                if tx.send((err, text)).is_err() {
                    return;
                }
            }
        }
        if !pending.is_empty() {
            let _ = tx.send((err, String::from_utf8_lossy(&pending).into_owned()));
        }
    })
}

/// [`run`] on a thread, for shells that can't block: `done` gets the result.
pub fn spawn(command: String, cwd: PathBuf, timeout: Duration, done: impl FnOnce(Result<ShellOutput>) + Send + 'static) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::spawn(move || done(run(&command, &cwd, timeout, &flag)));
    stop
}

/// `sh -c COMMAND` (`cmd /C` on Windows) in `cwd`, no input, both outputs piped, in a process
/// group of its own.
fn command_for(command: &str, cwd: &Path) -> Command {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };
    cmd.current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group, so a stop or time-out ends what it started too.
        cmd.process_group(0);
    }
    cmd
}

fn kill(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // The whole group: `sh` and whatever it started.
        let _ = Command::new("kill").args(["-KILL", &format!("-{}", child.id())]).stderr(Stdio::null()).status();
    }
    let _ = child.kill();
}

/// Reads a stream to the end on a thread, keeping its last [`MAX_OUTPUT`] bytes.
fn reader<R: Read + Send + 'static>(r: Option<R>) -> std::thread::JoinHandle<(String, bool)> {
    std::thread::spawn(move || {
        let Some(mut r) = r else { return (String::new(), false) };
        let mut kept: Vec<u8> = Vec::new();
        let mut cut = false;
        let mut buf = [0u8; 8192];
        while let Ok(n) = r.read(&mut buf) {
            if n == 0 {
                break;
            }
            kept.extend_from_slice(&buf[..n]);
            if kept.len() > 2 * MAX_OUTPUT {
                kept.drain(..kept.len() - MAX_OUTPUT);
                cut = true;
            }
        }
        if kept.len() > MAX_OUTPUT {
            kept.drain(..kept.len() - MAX_OUTPUT);
            cut = true;
        }
        (String::from_utf8_lossy(&kept).into_owned(), cut)
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn go(cmd: &str, dir: &Path) -> ShellOutput {
        run(cmd, dir, Duration::from_secs(20), &AtomicBool::new(false)).unwrap()
    }

    #[test]
    fn parses_bang_and_slash_forms() {
        assert_eq!(parse("!ls -la"), Some("ls -la"));
        assert_eq!(parse("  ! git status "), Some("git status"));
        assert_eq!(parse("/shell cargo test"), Some("cargo test"));
        assert_eq!(parse("/sh pwd"), Some("pwd"));
        assert_eq!(parse("!"), None);
        assert_eq!(parse("/shell"), None);
        assert_eq!(parse("/shellfish x"), None);
        assert_eq!(parse("tidy this"), None);
        assert!(is_shell_line("/shell") && is_shell_line("!") && !is_shell_line("/shellfish") && !is_shell_line("hello"));
    }

    #[test]
    fn runs_in_the_project_folder_with_both_streams_and_the_code() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("marker.txt"), "x").unwrap();
        let o = go("ls; echo oops >&2; exit 3", d.path());
        assert!(o.stdout.contains("marker.txt"), "{o:?}");
        assert_eq!(o.stderr.trim(), "oops");
        assert_eq!(o.code, Some(3));
        assert!(!o.ok());
        assert!(o.status().starts_with("exit 3"));
        let t = o.transcript();
        assert!(t.starts_with("$ ls; echo oops >&2; exit 3\n") && t.contains("oops\n") && t.contains("[exit 3"), "{t}");
        assert!(go("true", d.path()).ok());
    }

    #[test]
    fn a_time_limit_and_a_stop_end_it() {
        let d = tempfile::tempdir().unwrap();
        let o = run("sleep 30", d.path(), Duration::from_millis(200), &AtomicBool::new(false)).unwrap();
        assert!(o.timed_out && o.code.is_none() && o.elapsed_ms < 10_000, "{o:?}");
        let stop = AtomicBool::new(true);
        let o = run("sleep 30", d.path(), Duration::from_secs(60), &stop).unwrap();
        assert!(o.stopped && o.status().starts_with("stopped"), "{o:?}");
    }

    #[test]
    fn long_output_keeps_its_end() {
        let d = tempfile::tempdir().unwrap();
        let o = go("seq 1 100000", d.path());
        assert!(o.truncated && o.stdout.len() <= MAX_OUTPUT);
        assert!(o.stdout.ends_with("100000\n"));
    }

    #[test]
    fn a_missing_folder_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let e = run("true", &d.path().join("nope"), Duration::from_secs(5), &AtomicBool::new(false)).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Storage);
    }

    #[test]
    fn streaming_hands_over_output_and_stops() {
        let d = tempfile::tempdir().unwrap();
        let mut got = Vec::new();
        let e =
            run_streaming("echo one; echo two >&2; exit 4", d.path(), Duration::from_secs(10), &|| false, &mut |err, t| got.push((err, t))).unwrap();
        assert_eq!(e.code, Some(4));
        let out: String = got.iter().filter(|(e, _)| !e).map(|(_, t)| t.as_str()).collect();
        let err: String = got.iter().filter(|(e, _)| *e).map(|(_, t)| t.as_str()).collect();
        assert_eq!((out.as_str(), err.as_str()), ("one\n", "two\n"));
        let t0 = Instant::now();
        let e = run_streaming("sleep 30", d.path(), Duration::from_secs(60), &|| t0.elapsed() > Duration::from_millis(200), &mut |_, _| {}).unwrap();
        assert!(e.stopped && e.code.is_none() && e.elapsed_ms < 10_000);
    }

    #[test]
    fn spawn_reports_back() {
        let d = tempfile::tempdir().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        spawn("echo hi".into(), d.path().to_path_buf(), Duration::from_secs(10), move |r| tx.send(r).unwrap());
        let o = rx.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
        assert_eq!(o.stdout, "hi\n");
    }
}
