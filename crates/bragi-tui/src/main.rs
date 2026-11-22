//! `bragi-tui`: the terminal app under its old name. `bragi` runs the same code in-process;
//! this binary stays for one release so scripts and habits keep working.

use std::process::ExitCode;

fn main() -> ExitCode {
    bragi_tui::entry::run("bragi-tui", std::env::args().skip(1).collect())
}
