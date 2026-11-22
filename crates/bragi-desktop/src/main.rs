//! `bragi`: the desktop shell. `--demo` runs against an in-process demo inference node with a
//! test identity (no real network); `--scenario` also plays the shared end-to-end scenario
//! (pair, list models, chat, cancel, continue) through the view's own actions and exits 0 when
//! it passes.

// Release builds on Windows are GUI-subsystem apps: no console window opens beside Bragi.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// Copied whole from Morph and kept in step until they move to a shared weirdnet-* UI crate.
mod markdown;
#[allow(dead_code)]
mod text_input;
mod theme;
mod view;

use std::path::PathBuf;
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!(
        "usage: bragi [--home DIR] [--demo] [--scenario]\n       bragi mcp [--help]   Bragi as an MCP server for Cursor, Claude Desktop, VS Code"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    // `bragi mcp …`: the MCP server, without opening a window.
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        return bragi_mcp::cli::main(std::env::args().skip(2));
    }
    let mut home: Option<PathBuf> = std::env::var_os("BRAGI_HOME").map(PathBuf::from);
    let (mut demo, mut scenario) = (false, false);
    let mut args = std::env::args().skip(1);
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
            "--version" => {
                println!("bragi {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            _ => return usage(),
        }
    }
    view::run(view::Launch { home, demo, scenario })
}
