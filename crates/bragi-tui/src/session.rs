//! Opening Bragi for the terminal: the profile folder (its own, so the terminal app is a device
//! of its own and never shares an identity with a desktop Bragi on the same machine), a lock so
//! two `bragi-tui`s don't run one profile, and the demo profile.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bragi_core::demo::DemoNode;
use bragi_core::{Config, Core, PairState, Transport};

/// Where `bragi-tui` keeps this device's Bragi data unless `--home` or `BRAGI_TUI_HOME` says.
pub fn default_home() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("bragi-tui")
}

/// What the other device calls this one while linking.
pub fn label() -> String {
    let host = std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_default();
    if host.is_empty() { "Bragi terminal".into() } else { format!("Bragi terminal on {host}") }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Launch {
    pub home: Option<PathBuf>,
    /// The in-process demo nodes with a test identity: no real network.
    pub demo: bool,
}

/// One profile in use: removed when dropped.
struct Lock(PathBuf);

impl Lock {
    fn take(dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join("bragi-tui.lock");
        for _ in 0..2 {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut f) => {
                    use std::io::Write;
                    let _ = write!(f, "{}", std::process::id());
                    return Ok(Self(path));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let pid = std::fs::read_to_string(&path).unwrap_or_default();
                    let alive = Path::new("/proc").is_dir() && Path::new("/proc").join(pid.trim()).exists() && !pid.trim().is_empty();
                    // Without /proc (macOS, Windows) a stale lock can't be told apart: say how to clear it.
                    if alive || !Path::new("/proc").is_dir() {
                        return Err(format!(
                            "another bragi-tui is using {} (pid {}); quit it, use --home for another profile, or remove {} if it isn't running",
                            dir.display(),
                            pid.trim(),
                            path.display()
                        ));
                    }
                    let _ = std::fs::remove_file(&path);
                }
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
        Err(format!("couldn't lock {}", path.display()))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A running core and what keeps it alive.
pub struct Session {
    pub core: Core,
    /// The profile folder (the undo journal for code changes lives here too).
    pub home: PathBuf,
    pub demo: Option<Arc<DemoNode>>,
    _lock: Option<Lock>,
    _scratch: Option<tempfile::TempDir>,
    // Last: the core's tasks run on it.
    _runtime: Option<tokio::runtime::Runtime>,
}

impl Session {
    pub fn open(launch: &Launch) -> Result<Self, String> {
        if launch.demo {
            return Self::demo(launch.home.clone());
        }
        let home = launch.home.clone().or_else(|| std::env::var_os("BRAGI_TUI_HOME").map(PathBuf::from)).unwrap_or_else(default_home);
        let lock = Lock::take(&home)?;
        let mut cfg = Config::new(home.clone(), label());
        cfg.transport = Transport::Internet;
        let core = Core::start(cfg).map_err(|e| e.message)?;
        Ok(Self { core, home, demo: None, _lock: Some(lock), _scratch: None, _runtime: None })
    }

    /// The demo: two in-process inference nodes of one test user (memory transport), and a
    /// throwaway profile unless `home` names one. The demo node accepts link codes by itself.
    pub fn demo(home: Option<PathBuf>) -> Result<Self, String> {
        let (home, scratch) = match home {
            Some(h) => (h, None),
            None => {
                let d = tempfile::tempdir().map_err(|e| e.to_string())?;
                (d.path().to_path_buf(), Some(d))
            }
        };
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(|e| e.to_string())?;
        let node = rt.block_on(DemoNode::start_cloud(&home.join("demo"), Duration::from_millis(15))).map_err(|e| e.message)?;
        let core = {
            let _g = rt.enter();
            Core::start(node.app_config(&home.join("app"), "Bragi terminal (demo)")).map_err(|e| e.message)?
        };
        node.auto_accept_after(&core, Duration::from_millis(600));
        Ok(Self { core, home, demo: Some(node), _lock: None, _scratch: scratch, _runtime: Some(rt) })
    }

    /// Waits until this device is linked, online and sees at least one model (the demo links
    /// itself; a real profile must be linked first).
    pub fn ready(&self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        let mut offered = false;
        loop {
            let s = self.core.state();
            match &s.pair {
                PairState::Linked { .. } if s.online && !s.catalog.models.is_empty() => return Ok(()),
                PairState::Unpaired if self.demo.is_some() && !offered => {
                    offered = true;
                    let core = self.core.clone();
                    let _ = self.core.handle().block_on(async move { core.pair_offer().await });
                }
                PairState::Confirm { .. } if self.demo.is_some() => {
                    let _ = self.core.pair_confirm(true);
                }
                PairState::Unpaired if self.demo.is_none() => {
                    return Err("this terminal isn't linked to your Weirdnet yet: run `bragi-tui link` (or open `bragi-tui` and press Ctrl-L)".into());
                }
                PairState::Failed { message } => return Err(message.clone()),
                _ => {}
            }
            if Instant::now() > deadline {
                return Err(if s.online { self.core.no_node_message() } else { "timed out going online".into() });
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(d) = &self.demo {
            d.stop();
        }
        self.core.shutdown();
    }
}
