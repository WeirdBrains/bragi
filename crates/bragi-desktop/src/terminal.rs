//! `bragi PATH`: hands over to `bragi-tui`, which opens code mode on the path. It is looked
//! for beside this program first (the packages install both in one folder), then on the PATH.

use std::path::PathBuf;
use std::process::ExitCode;

const NAME: &str = if cfg!(windows) { "bragi-tui.exe" } else { "bragi-tui" };

fn beside_me() -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    let p = me.parent()?.join(NAME);
    p.is_file().then_some(p)
}

/// Runs `bragi-tui` with `args`; on Unix it replaces this process, so the terminal is its own.
pub fn run(args: Vec<String>) -> ExitCode {
    let program = beside_me().unwrap_or_else(|| PathBuf::from(NAME));
    let mut cmd = std::process::Command::new(&program);
    cmd.args(&args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let e = cmd.exec();
        eprintln!("bragi: can't start bragi-tui ({e}). Install it beside bragi to open a path in the terminal.");
        ExitCode::FAILURE
    }
    #[cfg(not(unix))]
    match cmd.status() {
        Ok(s) => ExitCode::from(s.code().unwrap_or(1) as u8),
        Err(e) => {
            eprintln!("bragi: can't start bragi-tui ({e}). Install it beside bragi to open a path in the terminal.");
            ExitCode::FAILURE
        }
    }
}
