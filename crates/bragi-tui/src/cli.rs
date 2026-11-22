//! The command line: the terminal app, and commands for scripts and SSH sessions (`link`,
//! `status`, `code`, `undo`).

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use bragi_core::code::diff::unified_all;
use bragi_core::code::{CodeRequest, CodeTask, Journal, parse_target};
use bragi_core::{ChatEvent, PairState};

use crate::code::CodeState;
use crate::session::{Launch, Session};

pub const USAGE: &str = "\
usage:
  bragi-tui [--home DIR] [--demo] [--root DIR]     the terminal app (F1 lists its keys)
  bragi-tui link [CODE | NODE-ADDRESS] [--yes]     link this terminal: show a code to open on a linked
                                                   device, open a code shown elsewhere, or add a node
                                                   address (ymirctl pool addresses)
  bragi-tui status                                 this device, inference nodes and models
  bragi-tui code --rewrite FILE[:FROM-TO] [options] HOW
  bragi-tui code --suggest [options] [WHAT]        suggestions across the --context files
      options: --context PATH (repeatable; folders respect .gitignore) --model ID
               --apply (write the change; `bragi-tui undo` puts it back)
      Prints the proposed change as a unified diff; the explanation goes to stderr.
  bragi-tui undo [--force]                         put back the last change `code --apply` or the app wrote
  bragi-tui --version | --help

  --rewrite/--suggest also work without `code`. The profile is BRAGI_TUI_HOME or
  ~/.local/share/bragi-tui: a device of its own, linked separately from a desktop Bragi.";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    pub launch: Launch,
    pub root: Option<PathBuf>,
    pub command: Cmd,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Cmd {
    #[default]
    Tui,
    Link {
        arg: Option<String>,
        yes: bool,
    },
    Status,
    Code {
        task: CodeTaskArg,
        context: Vec<String>,
        model: Option<String>,
        apply: bool,
        instruction: String,
    },
    Undo {
        force: bool,
    },
    Version,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum CodeTaskArg {
    Rewrite(String),
    #[default]
    Suggest,
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut out = Args::default();
    let mut words: Vec<String> = Vec::new();
    let (mut task, mut context, mut model, mut apply, mut yes, mut force) = (None, Vec::new(), None, false, false, false);
    let mut it = args.into_iter();
    let mut code = false;
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "--home" => out.launch.home = Some(value("--home")?.into()),
            "--root" => out.root = Some(value("--root")?.into()),
            "--demo" => out.launch.demo = true,
            "--rewrite" => {
                task = Some(CodeTaskArg::Rewrite(value("--rewrite")?));
                code = true;
            }
            "--suggest" => {
                task = Some(CodeTaskArg::Suggest);
                code = true;
            }
            "--context" | "-c" => context.push(value("--context")?),
            "--model" | "-m" => model = Some(value("--model")?),
            "--apply" => apply = true,
            "--yes" | "-y" => yes = true,
            "--force" => force = true,
            "--version" | "-V" => out.command = Cmd::Version,
            "--help" | "-h" => out.command = Cmd::Help,
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            _ => words.push(a),
        }
    }
    if matches!(out.command, Cmd::Version | Cmd::Help) {
        return Ok(out);
    }
    let first = words.first().map(String::as_str);
    out.command = match first {
        Some("code") => {
            words.remove(0);
            code_cmd(task, context, model, apply, words)?
        }
        _ if code => code_cmd(task, context, model, apply, words)?,
        Some("link") => {
            if words.len() > 2 {
                return Err("link takes one code or address".into());
            }
            Cmd::Link { arg: words.get(1).cloned(), yes }
        }
        Some("status") if words.len() == 1 => Cmd::Status,
        Some("undo") if words.len() == 1 => Cmd::Undo { force },
        Some("tui") if words.len() == 1 => Cmd::Tui,
        None => Cmd::Tui,
        Some(w) => return Err(format!("unknown command {w}")),
    };
    Ok(out)
}

fn code_cmd(task: Option<CodeTaskArg>, context: Vec<String>, model: Option<String>, apply: bool, words: Vec<String>) -> Result<Cmd, String> {
    let task = task.ok_or("code needs --rewrite FILE[:FROM-TO] or --suggest")?;
    let instruction = words.join(" ");
    if matches!(task, CodeTaskArg::Rewrite(_)) && instruction.trim().is_empty() {
        return Err("say how to rewrite it: bragi-tui code --rewrite FILE:10-40 \"what to change\"".into());
    }
    Ok(Cmd::Code { task, context, model, apply, instruction })
}

/// Runs a non-interactive command; returns the exit code.
pub fn run(args: Args) -> i32 {
    let result = match args.command.clone() {
        Cmd::Version => {
            println!("bragi-tui {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Cmd::Help => {
            println!("{USAGE}");
            Ok(())
        }
        Cmd::Undo { force } => undo(&args, force),
        Cmd::Link { arg, yes } => Session::open(&args.launch).and_then(|s| link(&s, arg, yes)),
        Cmd::Status => Session::open(&args.launch).and_then(|s| status(&s)),
        Cmd::Code { task: CodeTaskArg::Suggest, context, .. } if context.is_empty() => {
            Err("--suggest needs files: add --context PATH (a file or folder)".into())
        }
        Cmd::Code { task, context, model, apply, instruction } => {
            Session::open(&args.launch).and_then(|s| code(&s, root(&args), task, context, model, apply, instruction))
        }
        Cmd::Tui => Err("the terminal app is started by main".into()),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("bragi-tui: {e}");
            1
        }
    }
}

pub fn root(args: &Args) -> PathBuf {
    args.root.clone().or_else(|| std::env::current_dir().ok()).unwrap_or_else(|| PathBuf::from("."))
}

fn home_of(args: &Args) -> PathBuf {
    args.launch.home.clone().or_else(|| std::env::var_os("BRAGI_TUI_HOME").map(PathBuf::from)).unwrap_or_else(crate::session::default_home)
}

fn undo(args: &Args, force: bool) -> Result<(), String> {
    let undone = Journal::new(home_of(args).join("code-undo")).undo_last(force).map_err(|e| e.message)?;
    for f in &undone.files {
        println!("restored {}", f.display());
    }
    Ok(())
}

fn ask(question: &str) -> bool {
    eprint!("{question} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    matches!(line.trim(), "y" | "Y" | "yes")
}

fn link(s: &Session, arg: Option<String>, yes: bool) -> Result<(), String> {
    let core = &s.core;
    if let Some(a) = arg.as_deref().map(str::trim).filter(|a| a.starts_with('/')) {
        core.add_peer(a).map_err(|e| e.message)?;
        println!("Added {a}; this device dials it from now on.");
        return Ok(());
    }
    // Give a linked device a moment to come up before deciding it isn't.
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && !matches!(core.state().pair, PairState::Linked { .. }) && core.has_identity() {
        std::thread::sleep(Duration::from_millis(50));
    }
    if let PairState::Linked { device, .. } = core.state().pair
        && arg.is_none()
    {
        println!("Already linked (device {device}).");
        return Ok(());
    }
    let rt = core.handle().clone();
    if let Some(code) = arg {
        let check = rt.block_on(core.pair_accept(&code)).map_err(|e| e.message)?;
        let ok = yes || ask(&format!("Does {} show {}?", check.device, check.sas));
        rt.block_on(core.pair_accept_confirm(ok)).map_err(|e| e.message)?;
        println!("{}", if ok { "Linked." } else { "Cancelled." });
        return Ok(());
    }
    let link = rt.block_on(core.pair_offer()).map_err(|e| e.message)?;
    println!("{link}");
    if let Ok(qr) = qrcode::QrCode::new(link.as_bytes()) {
        let art = qr
            .render::<qrcode::render::unicode::Dense1x2>()
            .quiet_zone(true)
            .dark_color(qrcode::render::unicode::Dense1x2::Light)
            .light_color(qrcode::render::unicode::Dense1x2::Dark)
            .build();
        eprintln!("{art}");
    }
    eprintln!("Open this code on a device that's already linked (Ymir: ymirctl pool add CODE; Bragi: Link a device).");
    loop {
        match core.state().pair {
            PairState::Confirm { sas } => {
                let ok = yes || ask(&format!("Does the other device show {sas}?"));
                core.pair_confirm(ok).map_err(|e| e.message)?;
                if !ok {
                    println!("Cancelled.");
                    return Ok(());
                }
            }
            PairState::Linked { .. } => {
                println!("Linked.");
                return Ok(());
            }
            PairState::Failed { message } => return Err(message),
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn status(s: &Session) -> Result<(), String> {
    let core = &s.core;
    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        let st = core.state();
        let up = st.online && !st.catalog.models.is_empty();
        if up || (!core.has_identity() && s.demo.is_none()) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let st = core.state();
    let pair = match &st.pair {
        PairState::Linked { device, .. } => format!("linked (device {device})"),
        PairState::Unpaired => "not linked: run `bragi-tui link`".into(),
        PairState::Failed { message } => format!("failed: {message}"),
        _ => "linking".into(),
    };
    println!("link:      {pair}");
    println!("online:    {}", if st.online { "yes" } else { "no" });
    if let Some(p) = core.peer_id() {
        println!("peer id:   {p}");
    }
    println!("nodes:     {}", st.catalog.nodes.len());
    for n in &st.catalog.nodes {
        println!("  {}  {} chat models, {} warm", n.id, n.models, n.warm);
    }
    if st.catalog.nodes.is_empty() && matches!(st.pair, PairState::Linked { .. }) {
        println!("  {}", core.no_node_message());
    }
    println!("models:    {}", st.catalog.models.len());
    for m in &st.catalog.models {
        println!("  {}  {:?}", m.id, m.status);
    }
    for p in core.peers() {
        println!("dials:     {p}");
    }
    Ok(())
}

fn code(
    s: &Session,
    root: PathBuf,
    task: CodeTaskArg,
    context: Vec<String>,
    model: Option<String>,
    apply: bool,
    instruction: String,
) -> Result<(), String> {
    let mut state = CodeState::new(root, s.home.join("code-undo"));
    if !context.is_empty() {
        state.add(&context)?;
    }
    let req: CodeRequest = match task {
        CodeTaskArg::Rewrite(t) => {
            let target = parse_target(&t).map_err(|e| e.message)?;
            match state.parse(&format!("/rewrite {t} {instruction}"))? {
                crate::code::Command::Ask(r) => r,
                _ => return Err(format!("can't rewrite {}", target.path)),
            }
        }
        CodeTaskArg::Suggest => {
            if state.gathered.files.is_empty() {
                return Err("--suggest needs files: add --context PATH (a file or folder)".into());
            }
            CodeRequest { task: CodeTask::Suggest, instruction, files: state.gathered.files.clone() }
        }
    };
    s.ready(Duration::from_secs(45))?;
    let messages = CodeState::messages(&req)?;
    let mut h = s.core.complete(messages, model).map_err(|e| e.message)?;
    let rt = s.core.handle().clone();
    let mut text = String::new();
    loop {
        match rt.block_on(h.events.recv()) {
            Some(ChatEvent::Delta { offset, text: t }) => {
                if let Some((i, _)) = text.char_indices().nth(offset as usize) {
                    text.truncate(i);
                }
                text.push_str(&t);
            }
            Some(ChatEvent::Resumed { offset, .. }) => {
                if let Some((i, _)) = text.char_indices().nth(offset as usize) {
                    text.truncate(i);
                }
            }
            Some(ChatEvent::Done { .. }) => break,
            Some(ChatEvent::Error { message, .. }) => return Err(message),
            Some(_) => {}
            None => return Err("the answer stopped".into()),
        }
    }
    state.start(req);
    let note = state.answered(&text);
    if !state.explanation.is_empty() {
        eprintln!("{}", state.explanation);
    }
    if state.diffs.is_empty() {
        eprintln!("{note}");
        return Ok(());
    }
    print!("{}", unified_all(&state.diffs));
    let _ = std::io::stdout().flush();
    if apply {
        eprintln!("{}", state.write()?.replace("/undo", "`bragi-tui undo`"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Args, String> {
        parse(s.split_whitespace().map(str::to_string))
    }

    #[test]
    fn parses_commands() {
        assert_eq!(p("").unwrap().command, Cmd::Tui);
        assert!(p("--demo").unwrap().launch.demo);
        assert_eq!(p("link").unwrap().command, Cmd::Link { arg: None, yes: false });
        assert_eq!(p("link /ip4/1.2.3.4/tcp/1/p2p/X").unwrap().command, Cmd::Link { arg: Some("/ip4/1.2.3.4/tcp/1/p2p/X".into()), yes: false });
        assert_eq!(p("undo --force").unwrap().command, Cmd::Undo { force: true });
        let a = p("--rewrite src/a.rs:10-40 make it faster --apply").unwrap();
        assert_eq!(
            a.command,
            Cmd::Code {
                task: CodeTaskArg::Rewrite("src/a.rs:10-40".into()),
                context: vec![],
                model: None,
                apply: true,
                instruction: "make it faster".into()
            }
        );
        assert_eq!(
            p("code --suggest -c src -m qwen").unwrap().command,
            Cmd::Code {
                task: CodeTaskArg::Suggest,
                context: vec!["src".into()],
                model: Some("qwen".into()),
                apply: false,
                instruction: String::new()
            }
        );
        assert!(p("code").is_err());
        assert!(p("--rewrite a.rs").is_err());
        assert!(p("frob").is_err());
        assert!(p("--bogus").is_err());
        assert_eq!(p("--version").unwrap().command, Cmd::Version);
    }
}
