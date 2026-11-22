//! The local API server (`bragi serve`) that editors use: the terminal app makes sure one is
//! running when it starts, in the background, and shows its state in the status bar. It is
//! `bragi_serve::ensure`: a server that is already there is reused and never stopped, and one
//! this app starts stops itself after 30 idle minutes.
//!
//! Off with `--no-serve` or `BRAGI_TUI_SERVE=off`. Its home is `BRAGI_HOME` (or the desktop
//! app's folder), the same one the editor extension reads the token from; the port is
//! `BRAGI_SERVE_PORT` or 4877.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bragi_serve::ensure::{self, Outcome, Probe};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServeState {
    /// Turned off (`--no-serve`, `BRAGI_TUI_SERVE=off`).
    Off,
    Starting,
    /// Answering; `started` when this app started it.
    Running {
        started: bool,
    },
    /// Was running and has stopped (idle shutdown).
    Stopped,
    Failed(String),
}

impl ServeState {
    /// The short word for the status bar.
    pub fn label(&self) -> &'static str {
        match self {
            ServeState::Off => "serve off",
            ServeState::Starting => "serve starting",
            ServeState::Running { .. } => "serve",
            ServeState::Stopped => "serve idle",
            ServeState::Failed(_) => "serve failed",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            ServeState::Failed(m) => m.clone(),
            ServeState::Running { started: true } => "bragi serve was started for the editors".into(),
            ServeState::Running { started: false } => "bragi serve is running".into(),
            ServeState::Stopped => "bragi serve stopped after being idle; it starts again with the next bragi-tui or bragi serve ensure".into(),
            ServeState::Starting => "starting bragi serve".into(),
            ServeState::Off => "bragi serve isn't started by this app".into(),
        }
    }
}

pub type Shared = Arc<Mutex<ServeState>>;

/// Whether the environment turns the autostart off.
pub fn env_off(v: Option<&str>) -> bool {
    matches!(v.map(|s| s.trim().to_ascii_lowercase()).as_deref(), Some("off" | "0" | "false" | "no"))
}

fn port() -> u16 {
    std::env::var("BRAGI_SERVE_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(bragi_serve::DEFAULT_PORT)
}

/// Starts the check in the background and returns the state it keeps up to date. With `off` it
/// does nothing.
pub fn start(off: bool) -> Shared {
    if off || env_off(std::env::var("BRAGI_TUI_SERVE").ok().as_deref()) {
        return Arc::new(Mutex::new(ServeState::Off));
    }
    let state: Shared = Arc::new(Mutex::new(ServeState::Starting));
    let s = state.clone();
    let spawned = std::thread::Builder::new().name("bragi-serve-ensure".into()).spawn(move || {
        let home = bragi_serve::cli::default_home();
        let port = port();
        let set = |v: ServeState| {
            if let Ok(mut g) = s.lock() {
                *g = v;
            }
        };
        match ensure::ensure(&ensure::Options::new(home.clone(), port)) {
            Ok(Outcome::Started { .. }) => set(ServeState::Running { started: true }),
            Ok(Outcome::Reused { .. }) => set(ServeState::Running { started: false }),
            Err(e) => set(ServeState::Failed(e.to_string())),
        }
        // Keep the status honest (a started server stops itself when idle). Never restarts it.
        loop {
            std::thread::sleep(Duration::from_secs(20));
            let was_failed = matches!(s.lock().map(|g| g.clone()), Ok(ServeState::Failed(_)));
            if was_failed {
                continue;
            }
            match ensure::probe(&home, port) {
                Probe::Ready { .. } => {
                    if let Ok(mut g) = s.lock()
                        && !matches!(*g, ServeState::Running { .. })
                    {
                        *g = ServeState::Running { started: false };
                    }
                }
                Probe::Down => set(ServeState::Stopped),
                _ => {}
            }
        }
    });
    if let Err(e) = spawned
        && let Ok(mut g) = state.lock()
    {
        *g = ServeState::Failed(e.to_string());
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_can_turn_it_off() {
        assert!(env_off(Some("off")));
        assert!(env_off(Some(" 0 ")));
        assert!(env_off(Some("False")));
        assert!(!env_off(Some("on")));
        assert!(!env_off(None));
    }

    #[test]
    fn off_starts_nothing() {
        let s = start(true);
        assert_eq!(*s.lock().unwrap(), ServeState::Off);
        assert_eq!(ServeState::Off.label(), "serve off");
    }
}
