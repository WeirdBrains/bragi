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
use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste};
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
    let serve = crate::serve::start(args.no_serve || args.launch.demo);
    match run_app(&session, root, opened, serve) {
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

fn run_app(session: &Session, root: std::path::PathBuf, opened: Option<cli::Opened>, serve: crate::serve::Shared) -> Result<(), String> {
    let mut terminal = ratatui::try_init().map_err(|e| e.to_string())?;
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    let mut app = App::new(session.core.clone(), &session.home, root);
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
            if app.quit {
                return Ok(());
            }
            if event::poll(Duration::from_millis(50))? {
                app.on_event(event::read()?);
                while event::poll(Duration::ZERO)? {
                    app.on_event(event::read()?);
                }
                dirty = true;
            }
        }
    })();
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    result.map_err(|e| e.to_string())
}
