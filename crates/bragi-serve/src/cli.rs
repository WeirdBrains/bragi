//! The `bragi serve` command line, shared by the `bragi` desktop binary (`bragi serve …`) and
//! the headless `bragi-serve` binary.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use bragi_core::demo::DemoNode;
use bragi_core::{Config, Core, PairState, Transport};

use crate::{DEFAULT_PORT, bind, load_or_create_token, serve, token_path};

pub const USAGE: &str = "usage:
  bragi serve [--home DIR] [--port PORT] [--demo]   serve the local API on 127.0.0.1
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
}

/// Runs `bragi serve` with the arguments after `serve`.
pub fn run(args: impl IntoIterator<Item = String>) -> ExitCode {
    let (mut home, mut port, mut demo, mut cmd) = (None::<PathBuf>, DEFAULT_PORT, false, Cmd::Serve);
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "token" => cmd = Cmd::Token,
            "token-path" => cmd = Cmd::TokenPath,
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
        Cmd::Serve => match serve_blocking(home, port, demo) {
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

fn serve_blocking(home: PathBuf, port: u16, demo: bool) -> anyhow::Result<()> {
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
        let shutdown = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        serve(core.clone(), token, listener, shutdown).await?;
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
