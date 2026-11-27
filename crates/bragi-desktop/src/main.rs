//! `bragi`: Bragi in one command. With no arguments, or just a path, it runs the terminal app
//! in-process (`bragi .` opens code mode on the folder). `bragi desktop` opens the GPUI window
//! (`--demo` runs it against an in-process demo inference node with a test identity;
//! `--scenario` also plays the shared end-to-end scenario through the view's own actions and
//! exits 0 when it passes; `--serve` also serves the local API for editors from the window's
//! core). `bragi serve …` runs that API alone, `bragi mcp …` is the MCP server. A build without
//! the `gui` feature (BSD, servers) has no window: `bragi` is still the terminal app.
//! `bragi-tui` is the old name of the terminal app, kept for one release.

// Copied whole from Morph and kept in step until they move to a shared weirdnet-* UI crate.
#[cfg(feature = "gui")]
mod audio;
#[cfg(feature = "gui")]
mod markdown;
#[cfg(feature = "gui")]
#[allow(dead_code)]
mod text_input;
#[cfg(feature = "gui")]
mod theme;
#[cfg(feature = "gui")]
mod video;
#[cfg(feature = "gui")]
mod view;

use std::process::ExitCode;

const USAGE: &str = "\
usage:
  bragi [PATH] [options]        the terminal app; with a folder or file it opens code mode there
  bragi desktop [--home DIR] [--demo] [--scenario] [--serve [--serve-port PORT]]
                                the desktop window (not in builds without the `gui` feature)
  bragi serve [ensure|status]   the local API for editors (`bragi serve --help`)
  bragi mcp                     Bragi as an MCP server for Cursor, Claude Desktop, VS Code
  bragi --version | --help

The terminal app's own usage follows. `bragi-tui` is the old name for it and still works.
";

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    /// `bragi mcp …`, with the arguments after `mcp`.
    Mcp(Vec<String>),
    /// `bragi serve …`, with the arguments after `serve`.
    Serve(Vec<String>),
    /// `bragi desktop …`, with the arguments after `desktop`.
    Desktop(Vec<String>),
    /// `bragi --help`: this usage, then the terminal app's.
    Help,
    /// Everything else, including nothing and a lone path: the terminal app.
    Terminal(Vec<String>),
}

/// Reads the command line without the program name. Only the first word picks a subcommand, so
/// a path named like one is written `./serve` or `--path serve`.
fn dispatch(args: &[String]) -> Action {
    let rest = || args[1..].to_vec();
    match args.first().map(String::as_str) {
        Some("mcp") => Action::Mcp(rest()),
        Some("serve") => Action::Serve(rest()),
        Some("desktop") => Action::Desktop(rest()),
        Some("--help" | "-h") => Action::Help,
        _ => Action::Terminal(args.to_vec()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match dispatch(&args) {
        Action::Mcp(rest) => bragi_mcp::cli::main(rest),
        Action::Serve(rest) => bragi_serve::cli::run(rest),
        Action::Desktop(rest) => desktop(rest),
        Action::Help => {
            print!("{USAGE}\n{}\n", bragi_tui::entry::usage("bragi"));
            ExitCode::SUCCESS
        }
        Action::Terminal(rest) => bragi_tui::entry::run("bragi", rest),
    }
}

#[cfg(not(feature = "gui"))]
fn desktop(_args: Vec<String>) -> ExitCode {
    eprintln!("bragi: this build has no desktop window; `bragi` is the terminal app here (bragi --help)");
    ExitCode::from(2)
}

#[cfg(feature = "gui")]
fn desktop(args: Vec<String>) -> ExitCode {
    use std::path::PathBuf;
    let mut home: Option<PathBuf> = std::env::var_os("BRAGI_HOME").map(PathBuf::from);
    let (mut demo, mut scenario, mut serve) = (false, false, None::<u16>);
    let usage = || {
        eprintln!("usage: bragi desktop [--home DIR] [--demo] [--scenario] [--serve [--serve-port PORT]]");
        ExitCode::from(2)
    };
    let mut args = args.into_iter();
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
    // Release builds on Windows are console programs (the terminal app needs the console);
    // the window drops the one it was started with.
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn FreeConsole() -> i32;
        }
        // SAFETY: no arguments, no pointers; it only detaches this process from its console.
        unsafe {
            FreeConsole();
        }
    }
    view::run(view::Launch { home, demo, scenario, serve })
}

#[cfg(test)]
mod tests {
    use super::{Action, dispatch};

    fn v(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn no_arguments_or_a_path_is_the_terminal_app() {
        assert_eq!(dispatch(&[]), Action::Terminal(vec![]));
        assert_eq!(dispatch(&v(".")), Action::Terminal(v(".")));
        assert_eq!(dispatch(&v("~/src/app --demo")), Action::Terminal(v("~/src/app --demo")));
        assert_eq!(dispatch(&v("src/main.rs")), Action::Terminal(v("src/main.rs")));
        // Its own commands and options stay with it.
        assert_eq!(dispatch(&v("status")), Action::Terminal(v("status")));
        assert_eq!(dispatch(&v("link --yes")), Action::Terminal(v("link --yes")));
        assert_eq!(dispatch(&v("--version")), Action::Terminal(v("--version")));
        assert_eq!(dispatch(&v("--demo --no-serve")), Action::Terminal(v("--demo --no-serve")));
    }

    #[test]
    fn subcommands_are_claimed_by_their_first_word() {
        assert_eq!(dispatch(&v("desktop")), Action::Desktop(vec![]));
        assert_eq!(dispatch(&v("desktop --demo --home /tmp/h")), Action::Desktop(v("--demo --home /tmp/h")));
        assert_eq!(dispatch(&v("mcp")), Action::Mcp(vec![]));
        assert_eq!(dispatch(&v("mcp --http --root .")), Action::Mcp(v("--http --root .")));
        assert_eq!(dispatch(&v("serve")), Action::Serve(vec![]));
        assert_eq!(dispatch(&v("serve ensure")), Action::Serve(v("ensure")));
        assert_eq!(dispatch(&v("serve status --port 1")), Action::Serve(v("status --port 1")));
    }

    #[test]
    fn a_path_named_like_a_subcommand_is_still_a_path() {
        for p in ["./serve", "./mcp", "./desktop", "--path serve", "--path desktop", "--demo mcp"] {
            assert_eq!(dispatch(&v(p)), Action::Terminal(v(p)), "{p}");
        }
    }

    #[test]
    fn help_is_the_combined_usage() {
        assert_eq!(dispatch(&v("--help")), Action::Help);
        assert_eq!(dispatch(&v("-h")), Action::Help);
        assert_eq!(dispatch(&v("serve --help")), Action::Serve(v("--help")));
        assert_eq!(dispatch(&v("mcp --help")), Action::Mcp(v("--help")));
    }
}
