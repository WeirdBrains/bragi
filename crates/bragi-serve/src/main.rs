//! `bragi-serve`: `bragi serve` without the desktop app (headless machines, CI).

fn main() -> std::process::ExitCode {
    bragi_serve::cli::run(std::env::args().skip(1))
}
