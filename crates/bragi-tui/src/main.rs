//! `bragi-tui`: Bragi in a terminal. With no command it opens the full-screen app; `link`,
//! `status`, `code` and `undo` run without it (see `--help`).

use std::io::Write;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use base64::Engine;
use bragi_tui::app::App;
use bragi_tui::cli::{self, Cmd, USAGE};
use bragi_tui::session::Session;
use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste};
use ratatui::crossterm::execute;

fn main() -> ExitCode {
    let args = match cli::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("bragi-tui: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if args.command != Cmd::Tui {
        return ExitCode::from(cli::run(args) as u8);
    }
    match bragi_tui::theme::resolve(&args.theme) {
        Ok(t) => bragi_tui::theme::init(t),
        Err(e) => {
            eprintln!("bragi-tui: {e}");
            return ExitCode::from(2);
        }
    }
    let session = match Session::open(&args.launch) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("bragi-tui: {e}");
            return ExitCode::FAILURE;
        }
    };
    match run(&session, cli::root(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("bragi-tui: {e}");
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

fn run(session: &Session, root: std::path::PathBuf) -> Result<(), String> {
    let mut terminal = ratatui::try_init().map_err(|e| e.to_string())?;
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    let mut app = App::new(session.core.clone(), &session.home, root);
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
                terminal.draw(|f| bragi_tui::ui::draw(f, &mut app))?;
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
