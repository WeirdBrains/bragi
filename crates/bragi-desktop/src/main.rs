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
#[allow(dead_code)]
mod text_input;
mod theme;
mod view;

use std::path::PathBuf;
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!(
        "usage: bragi [--home DIR] [--demo] [--scenario] [--serve [--serve-port PORT]]\n       bragi serve --help\n       bragi mcp --help     Bragi as an MCP server for Cursor, Claude Desktop, VS Code"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    // `bragi mcp …`: the MCP server, without opening a window.
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        return bragi_mcp::cli::main(std::env::args().skip(2));
    }
    let mut home: Option<PathBuf> = std::env::var_os("BRAGI_HOME").map(PathBuf::from);
    let (mut demo, mut scenario, mut serve) = (false, false, None::<u16>);
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().map(String::as_str) == Some("serve") {
        args.next();
        return bragi_serve::cli::run(args);
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
