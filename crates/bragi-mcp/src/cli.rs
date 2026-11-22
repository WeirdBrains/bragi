//! The `bragi mcp` command line, shared by the `bragi` desktop binary (`bragi mcp …`) and the
//! `bragi-mcp` binary.

use std::io::{BufRead, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bragi_core::demo::DemoNode;
use bragi_core::{AppState, Config, Core, PairState, Transport};
use bragi_serve::cli::default_home;
use bragi_serve::{load_or_create_token, token_path};

use crate::{BragiTools, DEFAULT_HTTP, HTTP_PATH, Options, loopback_addr, serve_http, serve_stdio, server};

pub const USAGE: &str = "usage:
  bragi mcp [--root DIR] [--http [ADDR]] [--allow-writes] [--allow-deletes] [--timeout SECS] [--home DIR] [--demo]
                                 serve MCP on stdin/stdout, or on http://ADDR/mcp (loopback only,
                                 default 127.0.0.1:4878) with a bearer token
  bragi mcp token [--home DIR]   print the bearer token for --http (made on first use)
  bragi mcp token-path [--home DIR]
  bragi mcp status [--home DIR]  linked or not, the nodes, the models and the endpoints
  bragi mcp link [CODE] [--home DIR]
                                 link a Bragi home that isn't linked yet (shows a code, or opens one)
  bragi mcp add-node ADDRESS [--home DIR]
                                 remember an inference node's address (`ymirctl pool addresses`)

The server uses the Bragi home the app uses ($BRAGI_HOME, else ~/.local/share/bragi), so chats
land in the same history, folders and endpoints. --root is the project folder files are attached
from and saved to (default: the current folder). The tool that saves a chat's files into it is
offered only with --allow-writes, and the tools that delete sessions or folders only with
--allow-deletes. --demo answers from an in-process demo node with a test identity and a throwaway
profile (no real network).";

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Serve,
    Link(Option<String>),
    AddNode(String),
    Status,
    Token,
    TokenPath,
    Help,
    Version,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    pub command: Command,
    pub home: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub http: Option<SocketAddr>,
    pub allow_writes: bool,
    pub allow_deletes: bool,
    pub timeout: Duration,
    pub demo: bool,
}

/// Parses the arguments after the program name (or after `mcp` for `bragi mcp`).
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut a = Args {
        command: Command::Serve,
        home: None,
        root: None,
        http: None,
        allow_writes: false,
        allow_deletes: false,
        timeout: Duration::from_secs(600),
        demo: false,
    };
    let mut it = args.into_iter().peekable();
    let mut first = true;
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "serve" if first => a.command = Command::Serve,
            "link" if first => a.command = Command::Link(None),
            "add-node" if first => a.command = Command::AddNode(it.next().ok_or("add-node needs an address")?),
            "status" if first => a.command = Command::Status,
            "token" if first => a.command = Command::Token,
            "token-path" if first => a.command = Command::TokenPath,
            "-h" | "--help" | "help" => a.command = Command::Help,
            "--version" => a.command = Command::Version,
            "--home" => a.home = Some(it.next().ok_or("--home needs a folder")?.into()),
            "--root" => a.root = Some(it.next().ok_or("--root needs a folder")?.into()),
            "--http" => {
                let v = match it.peek() {
                    Some(v) if !v.starts_with('-') => it.next().unwrap_or_default(),
                    _ => DEFAULT_HTTP.to_string(),
                };
                a.http = Some(loopback_addr(&v).map_err(|e| format!("{e:#}"))?);
            }
            "--allow-writes" => a.allow_writes = true,
            "--allow-deletes" => a.allow_deletes = true,
            "--timeout" => {
                let v = it.next().ok_or("--timeout needs seconds")?;
                let s: u64 = v.parse().map_err(|_| format!("--timeout wants seconds, not {v:?}"))?;
                a.timeout = Duration::from_secs(s.max(1));
            }
            "--demo" => a.demo = true,
            code if matches!(a.command, Command::Link(None)) && !code.starts_with('-') => a.command = Command::Link(Some(code.to_string())),
            other => return Err(format!("unknown argument {other:?}")),
        }
        first = false;
    }
    Ok(a)
}

fn label() -> String {
    let host = std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .unwrap_or_default();
    if host.is_empty() { "Bragi MCP".into() } else { format!("Bragi on {host}") }
}

async fn until(core: &Core, what: &str, wait: Duration, f: impl Fn(&AppState) -> bool) -> anyhow::Result<AppState> {
    let deadline = Instant::now() + wait;
    let mut w = core.watch();
    loop {
        let s = w.borrow_and_update().clone();
        if f(&s) {
            return Ok(s);
        }
        if let PairState::Failed { message } = &s.pair {
            anyhow::bail!("{message}");
        }
        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for {what}");
        }
        let _ = tokio::time::timeout(Duration::from_millis(250), w.changed()).await;
    }
}

/// A core linked to an in-process demo inference node (a test identity, the memory transport),
/// with its profile in `dir`.
pub async fn demo(dir: &Path) -> anyhow::Result<(Arc<DemoNode>, Core)> {
    let node = Arc::new(DemoNode::start(&dir.join("demo"), Duration::from_millis(5)).await?);
    let core = Core::start(node.app_config(&dir.join("app"), "Bragi MCP (demo)"))?;
    node.auto_accept(&core);
    core.pair_offer().await?;
    until(&core, "the demo link", Duration::from_secs(30), |s| matches!(s.pair, PairState::Confirm { .. })).await?;
    core.pair_confirm(true)?;
    until(&core, "the demo node", Duration::from_secs(30), |s| {
        matches!(s.pair, PairState::Linked { .. }) && s.online && !s.catalog.models.is_empty()
    })
    .await?;
    Ok((node, core))
}

fn confirm(question: &str) -> bool {
    eprint!("{question} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    matches!(line.trim(), "y" | "Y" | "yes")
}

async fn link(core: &Core, code: Option<String>) -> anyhow::Result<()> {
    if let PairState::Linked { .. } = core.state().pair {
        println!("Already linked.");
        return Ok(());
    }
    let accepting = code.is_some();
    let sas = match code {
        Some(code) => core.pair_accept(&code).await?.sas,
        None => {
            let link = core.pair_offer().await?;
            println!("Open this code on a linked device (on a Ymir node: ymirctl pool add CODE):\n\n{link}\n");
            let s = until(core, "the other device", Duration::from_secs(600), |s| matches!(s.pair, PairState::Confirm { .. })).await?;
            let PairState::Confirm { sas } = s.pair else { anyhow::bail!("linking stopped") };
            sas
        }
    };
    let matches = confirm(&format!("Does the other device show {sas}?"));
    if accepting {
        core.pair_accept_confirm(matches).await?;
    } else {
        core.pair_confirm(matches)?;
    }
    if !matches {
        anyhow::bail!("the codes didn't match; nothing was linked");
    }
    until(core, "the link", Duration::from_secs(120), |s| matches!(s.pair, PairState::Linked { .. })).await?;
    println!("Linked.");
    Ok(())
}

/// A throwaway folder for --demo, removed on exit.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch_dir() -> anyhow::Result<Scratch> {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).map_err(|e| anyhow::anyhow!("no randomness: {e}"))?;
    let d = std::env::temp_dir().join(format!("bragi-mcp-demo-{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>()));
    std::fs::create_dir_all(&d)?;
    Ok(Scratch(d))
}

/// Runs a parsed command line.
pub async fn run(args: Args) -> anyhow::Result<()> {
    let home = args.home.clone().unwrap_or_else(default_home);
    match &args.command {
        Command::Help => {
            println!("{USAGE}");
            return Ok(());
        }
        Command::Version => {
            println!("bragi-mcp {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Command::TokenPath => {
            println!("{}", token_path(&home).display());
            return Ok(());
        }
        Command::Token => {
            println!("{}", load_or_create_token(&home)?);
            return Ok(());
        }
        _ => {}
    }
    let mut _scratch = None;
    let mut _demo = None;
    let mut home = home;
    let core = if args.demo {
        let d = scratch_dir()?;
        let (node, core) = demo(&d.0).await?;
        // Nothing of the demo goes in a real Bragi folder: its token and undo journal neither.
        home = d.0.join("home");
        _scratch = Some(d);
        _demo = Some(node);
        core
    } else {
        let mut cfg = Config::new(home.clone(), label());
        cfg.transport = Transport::Internet;
        let core = Core::start(cfg)?;
        // Endpoint API keys live where the app keeps them.
        core.use_os_keychain();
        core
    };
    let result = run_with(&core, &home, args).await;
    core.shutdown();
    result
}

async fn run_with(core: &Core, home: &Path, args: Args) -> anyhow::Result<()> {
    match args.command {
        Command::Link(code) => link(core, code).await,
        Command::AddNode(addr) => {
            core.add_peer(&addr)?;
            println!("Kept the address; Bragi dials it whenever it runs.");
            Ok(())
        }
        Command::Status => {
            let _ = until(core, "the nodes", Duration::from_secs(5), |s| s.online).await;
            let tools = BragiTools::new(core.clone(), Options::default())?;
            println!("{}", serde_json::to_string_pretty(&tools.status())?);
            Ok(())
        }
        Command::Serve => {
            let root = match args.root {
                Some(r) => r,
                None => std::env::current_dir()?,
            };
            let journal = crate::code::journal_dir(home);
            let opts = Options { allow_writes: args.allow_writes, allow_deletes: args.allow_deletes, root, timeout: args.timeout, journal };
            let srv = Arc::new(server(BragiTools::new(core.clone(), opts)?));
            let mode = match (args.allow_writes, args.allow_deletes) {
                (true, true) => ", writes and deletes allowed",
                (true, false) => ", writes allowed",
                (false, true) => ", deletes allowed",
                (false, false) => "",
            };
            match args.http {
                None => {
                    eprintln!("bragi mcp {}: MCP server on stdio{mode}", env!("CARGO_PKG_VERSION"));
                    serve_stdio(srv, tokio::io::stdin(), tokio::io::stdout()).await
                }
                Some(addr) => {
                    let token = load_or_create_token(home)?;
                    let listener = tokio::net::TcpListener::bind(addr).await?;
                    eprintln!(
                        "bragi mcp {}: MCP server on http://{}{HTTP_PATH}{mode} (bearer token: `bragi mcp token`)",
                        env!("CARGO_PKG_VERSION"),
                        listener.local_addr()?
                    );
                    serve_http(listener, srv, token, async {
                        let _ = tokio::signal::ctrl_c().await;
                    })
                    .await
                }
            }
        }
        Command::Help | Command::Version | Command::Token | Command::TokenPath => Ok(()),
    }
}

/// `bragi-mcp` and `bragi mcp`: parses `args` (after the program name, or after `mcp`) and runs.
pub fn main(args: impl IntoIterator<Item = String>) -> std::process::ExitCode {
    let args = match parse(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("bragi mcp: {e}\n\n{USAGE}");
            return std::process::ExitCode::from(2);
        }
    };
    let rt = match tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("bragi-mcp").enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("bragi mcp: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    match rt.block_on(run(args)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bragi mcp: {e:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Args, String> {
        parse(s.split_whitespace().map(str::to_string))
    }

    #[test]
    fn parses_the_command_line() {
        let a = p("").unwrap();
        assert_eq!(a.command, Command::Serve);
        assert!(!a.allow_deletes && !a.allow_writes && a.http.is_none());
        let a = p("serve --http 127.0.0.1:8765 --allow-deletes --allow-writes --root /tmp --timeout 30").unwrap();
        assert_eq!(a.http, Some("127.0.0.1:8765".parse().unwrap()));
        assert!(a.allow_deletes && a.allow_writes);
        assert_eq!(a.timeout, Duration::from_secs(30));
        assert_eq!(p("--http --demo").unwrap().http, Some(DEFAULT_HTTP.parse().unwrap()));
        assert_eq!(p("link weirdnet-bragi://link/abc").unwrap().command, Command::Link(Some("weirdnet-bragi://link/abc".into())));
        assert_eq!(p("link").unwrap().command, Command::Link(None));
        assert_eq!(p("token --home /tmp/x").unwrap().command, Command::Token);
        assert!(p("--http 0.0.0.0:8765").is_err());
        assert!(p("--bogus").is_err());
        assert!(p("serve link").is_err());
    }
}
