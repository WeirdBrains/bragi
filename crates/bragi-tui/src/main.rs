//! `bragi-tui`: Bragi in a terminal. With no command it opens the full-screen app; `link`,
//! `status`, `code` and `undo` run without it (see `--help`).

use std::io::Write;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use base64::Engine;
use bragi_tui::app::App;
use bragi_tui::cli::{self, Cmd, USAGE};
use bragi_tui::session::Session;
use ratatui::crossterm::event::{self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture};
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
    let (root, opened) = match cli::start(&args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("bragi-tui: {e}");
            return ExitCode::from(2);
        }
    };
    let mouse = args.mouse.or_else(|| cli::mouse_from_env(std::env::var("BRAGI_TUI_MOUSE").ok().as_deref())).unwrap_or(true);
    let serve = bragi_tui::serve::start(args.no_serve || args.launch.demo);
    match run(&session, root, opened, mouse, serve) {
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

/// Opens an address with the system's opener, detached and silent (web pages and mail only).
fn open_url(url: &str) {
    if !bragi_tui::app::openable(url) {
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

fn run(session: &Session, root: std::path::PathBuf, opened: Option<cli::Opened>, mouse: bool, serve: bragi_tui::serve::Shared) -> Result<(), String> {
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
                terminal.draw(|f| bragi_tui::ui::draw(f, &mut app))?;
                last = Instant::now();
                dirty = false;
            }
            for text in std::mem::take(&mut app.clipboard) {
                osc52(&mut std::io::stdout(), &text)?;
            }
            for url in std::mem::take(&mut app.open_urls) {
                open_url(&url);
            }
            // The terminal's mouse reporting follows what the app wants (F7 flips it).
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
