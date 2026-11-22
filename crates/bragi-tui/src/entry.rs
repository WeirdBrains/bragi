//! The terminal app's entry point, shared by `bragi` (which runs it in-process) and the
//! `bragi-tui` compatibility binary: with no command it opens the full-screen app; `link`,
//! `status`, `code` and `undo` run without it (see `--help`).

use std::io::Write;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use crate::app::App;
use crate::cli::{self, Cmd, USAGE};
use crate::session::Session;
use base64::Engine;
use ratatui::crossterm::event::{self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture};
use ratatui::crossterm::execute;

/// The usage text with `prog` for the program's name (`bragi` or `bragi-tui`).
pub fn usage(prog: &str) -> String {
    USAGE.replace("bragi-tui", prog)
}

/// Runs the terminal app (or one of its commands) as `prog` with `args`, the command line
/// without the program name.
pub fn run(prog: &str, args: Vec<String>) -> ExitCode {
    let args = match cli::parse(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{prog}: {e}\n\n{}", usage(prog));
            return ExitCode::from(2);
        }
    };
    match args.command {
        Cmd::Version => {
            println!("{prog} {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Cmd::Help => {
            println!("{}", usage(prog));
            return ExitCode::SUCCESS;
        }
        Cmd::Tui => {}
        _ => return ExitCode::from(cli::run(args) as u8),
    }
    let session = match Session::open(&args.launch) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{prog}: {e}");
            return ExitCode::FAILURE;
        }
    };
    match crate::theme::resolve(&args.theme, &session.core.appearance()) {
        Ok(t) => crate::theme::init(t),
        Err(e) => {
            eprintln!("{prog}: {e}");
            return ExitCode::from(2);
        }
    }
    let (root, opened) = match cli::start(&args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{prog}: {e}");
            return ExitCode::from(2);
        }
    };
    let mouse = args.mouse.or_else(|| cli::mouse_from_env(std::env::var("BRAGI_TUI_MOUSE").ok().as_deref())).unwrap_or(true);
    let serve = crate::serve::start(args.no_serve || args.launch.demo);
    match run_app(&session, root, opened, mouse, serve) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{prog}: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Puts `text` on the terminal's clipboard with OSC 52: works locally and over SSH in most
/// terminals (inside tmux, with `set -g set-clipboard on`).
fn osc52(out: &mut impl Write, text: &str) -> std::io::Result<()> {
    let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    write!(out, "\x1b]52;c;{b64}\x07")?;
    out.flush()
}

/// Opens an address with the system's opener, detached and silent (web pages and mail only).
fn open_url(url: &str) {
    if !crate::app::openable(url) {
        return;
    }
    let (cmd, args): (&str, Vec<&str>) = if cfg!(target_os = "macos") {
        ("open", vec![url])
    } else if cfg!(windows) {
        ("explorer", vec![url])
    } else {
        ("xdg-open", vec![url])
    };
    let _ = std::process::Command::new(cmd)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut c| std::thread::spawn(move || c.wait()));
}

/// Gives the terminal back as it was: mouse reporting and bracketed paste off. Runs on exit
/// and, through the panic hook, when the app panics.
fn leave_modes() {
    let _ = execute!(std::io::stdout(), DisableMouseCapture, DisableBracketedPaste);
}

fn run_app(session: &Session, root: std::path::PathBuf, opened: Option<cli::Opened>, mouse: bool, serve: crate::serve::Shared) -> Result<(), String> {
    let mut terminal = ratatui::try_init().map_err(|e| e.to_string())?;
    // ratatui's panic hook restores the screen; this one also switches off the mouse first,
    // so a crash doesn't leave the shell reporting clicks as escape codes.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        leave_modes();
        previous(info);
    }));
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    let mut captured = false;
    let mut app = App::new(session.core.clone(), &session.home, root);
    app.mouse = mouse;
    app.serve = Some(serve);
    if let Some(o) = opened {
        app.open_code(&o);
    }
    let result = (|| -> std::io::Result<()> {
        // Draws only when something changed: input, a job or stream event, a toast coming or
        // going, a resize, or a frame of the spinner or countdown (every 100 ms while one runs).
        let mut dirty = true;
        let mut last = Instant::now();
        loop {
            dirty |= app.tick();
            if app.animating() && last.elapsed() >= Duration::from_millis(100) {
                dirty = true;
            }
            if dirty {
                terminal.draw(|f| crate::ui::draw(f, &mut app))?;
                last = Instant::now();
                dirty = false;
            }
            for text in std::mem::take(&mut app.clipboard) {
                osc52(&mut std::io::stdout(), &text)?;
            }
            for url in std::mem::take(&mut app.open_urls) {
                open_url(&url);
            }
            // The terminal's mouse reporting follows what the app wants (F8 flips it).
            if app.mouse != captured {
                captured = app.mouse;
                if captured {
                    execute!(std::io::stdout(), EnableMouseCapture)?;
                } else {
                    execute!(std::io::stdout(), DisableMouseCapture)?;
                }
            }
            if app.quit {
                return Ok(());
            }
            if event::poll(Duration::from_millis(50))? {
                dirty |= app.on_event(event::read()?);
                while event::poll(Duration::ZERO)? {
                    dirty |= app.on_event(event::read()?);
                }
            }
        }
    })();
    leave_modes();
    ratatui::restore();
    result.map_err(|e| e.to_string())
}
