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
  bragi-tui [PATH] [--home DIR] [--demo] [--no-serve] [--root DIR]  the terminal app (F1 lists its keys)
                                                   with a PATH it opens code mode there: a folder is
                                                   the root and its files the context; a file is
                                                   opened in its git repository (or its folder).
                                                   `bragi-tui .` works here; use ./link for a path
                                                   named like a command, or --path PATH
      [--theme NAME|FILE.toml] [--mode dark|light|system]
                                                   --no-serve (or BRAGI_TUI_SERVE=off) doesn't start
                                                   `bragi serve` for the editors; it is on otherwise
                                                   its look (weirdnet-themes: bragi, the default,
                                                   tokyo-night, catppuccin, gruvbox, dracula, nord,
                                                   one, solarized). NO_COLOR, BRAGI_TUI_COLOR
                                                   (truecolor|256|16|none) and BRAGI_TUI_ASCII apply
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
    /// A path to open in code mode (`bragi-tui PATH`, `--path PATH`), as typed.
    pub open: Option<String>,
    pub command: Cmd,
    pub theme: crate::theme::Choice,
    /// Don't make sure `bragi serve` is running.
    pub no_serve: bool,
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
            "--path" => {
                if out.open.is_some() {
                    return Err("give one path to open".into());
                }
                out.open = Some(value("--path")?);
            }
            "--demo" => out.launch.demo = true,
            "--no-serve" => out.no_serve = true,
            "--theme" => out.theme.theme = Some(value("--theme")?),
            "--mode" => out.theme.mode = Some(value("--mode")?),
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
        // A lone word that isn't a command is a path to open (`bragi-tui .`, `bragi-tui ~/src/app`).
        Some(w) if words.len() == 1 && out.open.is_none() => {
            out.open = Some(w.to_string());
            Cmd::Tui
        }
        Some(w) => return Err(format!("unknown command {w}")),
    };
    if out.open.is_some() && out.root.is_some() {
        return Err("give a path or --root, not both".into());
    }
    if out.open.is_some() && out.command != Cmd::Tui {
        return Err("a path opens the terminal app; it doesn't go with a command".into());
    }
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

/// Where code mode starts: its root and, when a file was named, that file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opened {
    pub root: PathBuf,
    /// Added to the context, relative to `root`; `.` for a folder.
    pub context: PathBuf,
    pub is_file: bool,
}

/// Resolves a path typed on the command line against `cwd`: `~`, `.`, `..` and symlinks are
/// resolved. A folder is the root; a file's root is its git repository (the nearest folder with
/// a `.git`) or else its folder.
pub fn resolve_open(raw: &str, cwd: &std::path::Path) -> Result<Opened, String> {
    if raw.trim().is_empty() {
        return Err("the path is empty".into());
    }
    let typed = crate::code::expand(raw);
    let full = if typed.is_absolute() { typed } else { cwd.join(typed) };
    let path = full.canonicalize().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("{raw} doesn't exist"),
        _ => format!("can't open {raw}: {e}"),
    })?;
    if path.is_dir() {
        return Ok(Opened { root: path, context: PathBuf::from("."), is_file: false });
    }
    let parent = path.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
    let root = parent.ancestors().find(|d| d.join(".git").exists()).map(PathBuf::from).unwrap_or(parent);
    let context = path.strip_prefix(&root).map(PathBuf::from).unwrap_or_else(|_| path.clone());
    Ok(Opened { root, context, is_file: true })
}

/// The folder code mode works in, and what to open there, for the terminal app.
pub fn start(args: &Args) -> Result<(PathBuf, Option<Opened>), String> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match &args.open {
        Some(raw) => {
            let o = resolve_open(raw, &cwd)?;
            Ok((o.root.clone(), Some(o)))
        }
        None => Ok((root(args), None)),
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
        let t = p("--theme nord --mode light").unwrap().theme;
        assert_eq!((t.theme.as_deref(), t.mode.as_deref()), (Some("nord"), Some("light")));
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
        assert!(p("--bogus").is_err());
        assert_eq!(p("--version").unwrap().command, Cmd::Version);
    }

    #[test]
    fn a_lone_word_is_a_path_unless_it_is_a_command() {
        assert_eq!(p(".").unwrap().open.as_deref(), Some("."));
        assert_eq!(p("~/src/app --demo").unwrap().open.as_deref(), Some("~/src/app"));
        assert_eq!(p("--path serve").unwrap().open.as_deref(), Some("serve"));
        assert_eq!(p("./link").unwrap().open.as_deref(), Some("./link"));
        assert_eq!(p("./link").unwrap().command, Cmd::Tui);
        assert_eq!(p("link").unwrap().command, Cmd::Link { arg: None, yes: false });
        assert_eq!(p("status").unwrap().open, None);
        assert_eq!(p("src").unwrap().command, Cmd::Tui);
        assert!(p("a b").is_err());
        assert!(p(". --root x").is_err());
        assert!(p("--path a --path b").is_err());
        assert!(p("--path a status").is_err());
        assert_eq!(p("--root x").unwrap().open, None);
    }

    #[test]
    fn resolves_the_path_to_open() {
        let d = tempfile::tempdir().unwrap();
        let base = d.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("repo/.git")).unwrap();
        std::fs::create_dir_all(base.join("repo/src")).unwrap();
        std::fs::create_dir_all(base.join("plain")).unwrap();
        std::fs::write(base.join("repo/src/a.rs"), "fn a() {}").unwrap();
        std::fs::write(base.join("plain/b.txt"), "b").unwrap();

        // A folder is the root, however it is spelled.
        for typed in [".", "./", "../repo", &base.join("repo").display().to_string()] {
            let o = resolve_open(typed, &base.join("repo")).unwrap();
            assert_eq!(
                (o.root.as_path(), o.context.as_path(), o.is_file),
                (base.join("repo").as_path(), std::path::Path::new("."), false),
                "{typed}"
            );
        }
        // A file is opened from its git root, or from its folder with none.
        let o = resolve_open("repo/src/a.rs", &base).unwrap();
        assert_eq!((o.root, o.context, o.is_file), (base.join("repo"), "src/a.rs".into(), true));
        let o = resolve_open("plain/b.txt", &base).unwrap();
        assert_eq!((o.root, o.context), (base.join("plain"), "b.txt".into()));
        // Missing and empty paths say so.
        assert_eq!(resolve_open("nope", &base).unwrap_err(), "nope doesn't exist");
        assert!(resolve_open("", &base).is_err());
    }

    #[test]
    fn a_leading_tilde_is_expanded_before_looking() {
        // Whatever HOME is, `~/` is looked up there, not under the current folder.
        let e = resolve_open("~/no-such-folder-for-the-bragi-tests", std::path::Path::new("/")).unwrap_err();
        assert_eq!(e, "~/no-such-folder-for-the-bragi-tests doesn't exist");
    }
}
