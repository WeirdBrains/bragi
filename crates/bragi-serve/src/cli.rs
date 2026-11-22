//! The `bragi serve` command line, shared by the `bragi` desktop binary (`bragi serve …`) and
//! the headless `bragi-serve` binary.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use bragi_core::demo::DemoNode;
use bragi_core::{Config, Core, PairState, Transport};

use crate::{Activity, DEFAULT_PORT, bind, ensure, load_or_create_token, serve_tracked, token_path};

pub const USAGE: &str = "usage:
  bragi serve [--home DIR] [--port PORT] [--demo]   serve the local API on 127.0.0.1
        [--idle-timeout MINUTES]                   stop after that long with no client (default: never)
  bragi serve ensure [--home DIR] [--port PORT]     start a server in the background if none is
                                                   running (reuses one that is); prints its state
  bragi serve status [--home DIR] [--port PORT]     is a server running? exit 0 if so
  bragi serve token [--home DIR]                    print the API token, to paste into an editor
  bragi serve token-path [--home DIR]               print where the token is kept

The API listens on loopback only. --demo answers from an in-process demo node with a test
identity and a throwaway profile (no real network).";

/// Bragi's data folder: `$BRAGI_HOME`, else `$XDG_DATA_HOME/bragi`, else `~/.local/share/bragi`
/// (the desktop app's).
pub fn default_home() -> PathBuf {
    if let Some(h) = std::env::var_os("BRAGI_HOME") {
        return h.into();
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("bragi")
}

fn label() -> String {
    let host = std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .unwrap_or_default();
    if host.is_empty() { "Bragi serve".into() } else { format!("Bragi on {host}") }
}

enum Cmd {
    Serve,
    Token,
    TokenPath,
    Ensure,
    Status,
}

/// Runs `bragi serve` with the arguments after `serve`.
pub fn run(args: impl IntoIterator<Item = String>) -> ExitCode {
    let (mut home, mut port, mut demo, mut cmd) = (None::<PathBuf>, DEFAULT_PORT, false, Cmd::Serve);
    let mut idle = 0u64;
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "token" => cmd = Cmd::Token,
            "token-path" => cmd = Cmd::TokenPath,
            "ensure" => cmd = Cmd::Ensure,
            "status" => cmd = Cmd::Status,
            "--idle-timeout" => match args.next().and_then(|p| p.parse().ok()) {
                Some(m) => idle = m,
                None => return usage(),
            },
            "--home" => match args.next() {
                Some(h) => home = Some(h.into()),
                None => return usage(),
            },
            "--port" => match args.next().and_then(|p| p.parse().ok()) {
                Some(p) => port = p,
                None => return usage(),
            },
            "--demo" => demo = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => return usage(),
        }
    }
    let home = home.unwrap_or_else(default_home);
    match cmd {
        Cmd::TokenPath => {
            println!("{}", token_path(&home).display());
            ExitCode::SUCCESS
        }
        Cmd::Token => match load_or_create_token(&home) {
            Ok(t) => {
                println!("{t}");
                ExitCode::SUCCESS
            }
            Err(e) => fail(e),
        },
        Cmd::Ensure => match ensure::ensure(&ensure::Options::new(home, port)) {
            Ok(o) => {
                println!(
                    "{}",
                    match o {
                        ensure::Outcome::Reused { version } => format!("bragi serve {version} is already running on port {port}"),
                        ensure::Outcome::Started { version } => format!("started bragi serve {version} on port {port}"),
                    }
                );
                ExitCode::SUCCESS
            }
            Err(e) => fail(e.into()),
        },
        Cmd::Status => match ensure::probe(&home, port) {
            ensure::Probe::Ready { version } => {
                println!("running: bragi serve {version} on port {port}");
                ExitCode::SUCCESS
            }
            ensure::Probe::OtherToken { .. } => {
                println!("a Bragi server on port {port} uses another Bragi home");
                ExitCode::FAILURE
            }
            ensure::Probe::NotBragi => {
                println!("port {port} is used by something else");
                ExitCode::FAILURE
            }
            ensure::Probe::Down => {
                println!("not running");
                ExitCode::FAILURE
            }
        },
        Cmd::Serve => match serve_blocking(home, port, demo, idle) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(e),
        },
    }
}

fn usage() -> ExitCode {
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

fn fail(e: anyhow::Error) -> ExitCode {
    eprintln!("bragi serve: {e:#}");
    ExitCode::FAILURE
}

fn serve_blocking(home: PathBuf, port: u16, demo: bool, idle_minutes: u64) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("bragi-serve").enable_all().build()?;
    rt.block_on(async move {
        let mut _scratch = None;
        let mut _demo = None;
        let (core, token_home) = if demo {
            let d = tempfile_dir()?;
            let dir = d.clone();
            _scratch = Some(scopeguard(d));
            let node = DemoNode::start_cloud(&dir.join("demo"), Duration::from_millis(20)).await?;
            let core = Core::start(node.app_config(&dir.join("app"), "Bragi serve (demo)"))?;
            node.auto_accept(&core);
            core.pair_offer().await?;
            wait(&core, |p| matches!(p, PairState::Confirm { .. })).await?;
            core.pair_confirm(true)?;
            wait(&core, |p| matches!(p, PairState::Linked { .. })).await?;
            _demo = Some(node);
            // The demo still uses the real token, so an editor set up once keeps working.
            (core, home.clone())
        } else {
            let mut cfg = Config::new(home.clone(), label());
            cfg.transport = Transport::Internet;
            (Core::start(cfg)?, home.clone())
        };
        let token = load_or_create_token(&token_home)?;
        let listener = bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).await?;
        eprintln!(
            "bragi serve: listening on http://{} (token in {}){}",
            listener.local_addr()?,
            token_path(&token_home).display(),
            if demo { ", demo node" } else { "" }
        );
        let activity = Activity::default();
        let idle_for = (idle_minutes > 0).then(|| Duration::from_secs(idle_minutes * 60));
        if let Some(d) = idle_for {
            eprintln!("bragi serve: stops after {} idle minute{}", d.as_secs() / 60, if d.as_secs() == 60 { "" } else { "s" });
        }
        let watch = activity.clone();
        let shutdown = async move {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                () = async {
                    match idle_for {
                        Some(d) => watch.idle(d).await,
                        None => std::future::pending().await,
                    }
                } => eprintln!("bragi serve: no client for a while, stopping"),
            }
        };
        // `server.pid` says a server is up (a stale one is told apart by nothing answering).
        let _ = ensure::write_pid_file(&token_home, listener.local_addr()?.port());
        let result = serve_tracked(core.clone(), token, listener, shutdown, Some(activity)).await;
        ensure::remove_pid_file(&token_home);
        result?;
        core.shutdown();
        Ok(())
    })
}

async fn wait(core: &Core, f: impl Fn(&PairState) -> bool) -> anyhow::Result<()> {
    let mut w = core.watch();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if f(&w.borrow_and_update().pair) {
            return Ok(());
        }
        anyhow::ensure!(tokio::time::Instant::now() < deadline, "the demo node did not link: {:?}", w.borrow().pair);
        let _ = tokio::time::timeout(Duration::from_millis(200), w.changed()).await;
    }
}

/// A fresh private directory under the system temp dir.
fn tempfile_dir() -> anyhow::Result<PathBuf> {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).map_err(|e| anyhow::anyhow!("{e}"))?;
    let d = std::env::temp_dir().join(format!("bragi-serve-demo-{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>()));
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

/// Removes the directory when dropped.
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scopeguard(p: PathBuf) -> RemoveOnDrop {
    RemoveOnDrop(p)
}
