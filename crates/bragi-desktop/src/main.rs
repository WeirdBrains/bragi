//! `bragi`: the desktop shell. `--demo` runs against an in-process demo inference node with a
//! test identity (no real network); `--scenario` also plays the shared end-to-end scenario
//! (pair, list models, chat, cancel, continue) through the view's own actions and exits 0 when
//! it passes. `--serve` also serves the local API for editors from this app's core, and
//! `bragi serve …` runs that API alone (no window; see `bragi serve --help`).

// Release builds on Windows are GUI-subsystem apps: no console window opens beside Bragi.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// Copied whole from Morph and kept in step until they move to a shared weirdnet-* UI crate.
mod audio;
mod markdown;
mod terminal;
#[allow(dead_code)]
mod text_input;
mod theme;
mod view;

use std::path::PathBuf;
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!(
        "usage: bragi [--home DIR] [--demo] [--scenario] [--serve [--serve-port PORT]]\n       bragi PATH            the terminal app, in code mode on PATH (a folder or a file; `bragi .`)\n       bragi serve --help"
    );
    ExitCode::from(2)
}

/// Whether the first argument is a path or a terminal-app command (`bragi .`, `bragi ~/src/app`,
/// `bragi status`) rather than one of this window's options: those all start with `-`. `--path`
/// is the explicit form, for a folder named like a command.
fn wants_terminal(args: &[String]) -> bool {
    let mut it = args.iter().map(String::as_str);
    while let Some(a) = it.next() {
        match a {
            "--path" => return true,
            "--home" | "--serve-port" => {
                it.next();
            }
            _ if a.starts_with('-') => {}
            _ => return true,
        }
    }
    false
}

fn main() -> ExitCode {
    let mut home: Option<PathBuf> = std::env::var_os("BRAGI_HOME").map(PathBuf::from);
    let (mut demo, mut scenario, mut serve) = (false, false, None::<u16>);
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().map(String::as_str) == Some("serve") {
        args.next();
        return bragi_serve::cli::run(args);
    }
    let rest: Vec<String> = std::env::args().skip(1).collect();
    if wants_terminal(&rest) {
        return terminal::run(rest);
    }
    while let Some(a) = args.next() {
        match a.as_str() {
            "--home" => match args.next() {
                Some(h) => home = Some(h.into()),
                None => return usage(),
            },
            "--demo" => demo = true,
            "--scenario" => {
                demo = true;
                scenario = true;
            }
            "--serve" => serve = serve.or(Some(bragi_serve::DEFAULT_PORT)),
            "--serve-port" => match args.next().and_then(|p| p.parse().ok()) {
                Some(p) => serve = Some(p),
                None => return usage(),
            },
            "--version" => {
                println!("bragi {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => return usage(),
        }
    }
    view::run(view::Launch { home, demo, scenario, serve })
}

#[cfg(test)]
mod tests {
    use super::wants_terminal;

    fn v(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn a_path_or_command_goes_to_the_terminal_app() {
        assert!(wants_terminal(&v(".")));
        assert!(wants_terminal(&v("~/src/app")));
        assert!(wants_terminal(&v("./serve")));
        assert!(wants_terminal(&v("status")));
        assert!(wants_terminal(&v("--path x")));
        assert!(wants_terminal(&v("--demo --home /tmp/h .")));
        assert!(!wants_terminal(&[]));
        assert!(!wants_terminal(&v("--demo")));
        assert!(!wants_terminal(&v("--home /tmp/h --serve --serve-port 9")));
    }
}
