//! `bragi wake`: this device's wake-up settings (B4) for scripts and SSH sessions. Push wake-ups
//! are on by default; the relay wake path (`/weirdnet/wake/1`) is opt-in and costs battery, so
//! turning it on asks first unless `--yes`.

use std::io::{BufRead, Write};

use bragi_core::wake::{RELAY_WAKE_COST, WakeSettings};

use crate::session::{Launch, Session};

pub const USAGE: &str = "\
  bragi wake [status]                    wake-up settings, relay connection and linked devices
  bragi wake on|off                      let linked devices wake this one (on by default)
  bragi wake relay on|off [--yes]        also keep a connection to your relay for wakes (off by default)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sub {
    Status,
    Push(bool),
    Relay { on: bool, yes: bool },
}

fn onoff(w: &str) -> Result<bool, String> {
    match w {
        "on" | "true" | "yes" => Ok(true),
        "off" | "false" | "no" => Ok(false),
        _ => Err(format!("expected on or off, not {w}")),
    }
}

pub fn parse(args: Vec<String>) -> Result<Sub, String> {
    let mut yes = false;
    let mut words: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--yes" | "-y" => yes = true,
            "--help" | "-h" => return Err(USAGE.trim_start().into()),
            _ if a.starts_with('-') => return Err(format!("unknown option {a}")),
            _ => words.push(a),
        }
    }
    match words.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | ["status"] => Ok(Sub::Status),
        [w @ ("on" | "off")] => Ok(Sub::Push(onoff(w)?)),
        ["relay", w] => Ok(Sub::Relay { on: onoff(w)?, yes }),
        _ => Err(format!("wake needs status, on, off or relay on|off\n{USAGE}")),
    }
}

pub fn run(launch: &Launch, sub: Sub) -> Result<(), String> {
    let s = Session::open(launch)?;
    let core = &s.core;
    let now = core.wake_settings();
    match sub {
        Sub::Status => {}
        Sub::Push(push) => core.set_wake_settings(WakeSettings { push, ..now }).map_err(|e| e.message)?,
        Sub::Relay { on, yes } => {
            if on && !now.relay_wake && !yes && !ask(&format!("{RELAY_WAKE_COST}\nTurn on relay wake?")) {
                println!("Relay wake stays off.");
                return Ok(());
            }
            core.set_wake_settings(WakeSettings { relay_wake: on, ..now }).map_err(|e| e.message)?
        }
    }
    let st = core.wake_status();
    println!("wake-ups    {}", if st.settings.push { "on" } else { "off" });
    let relay = match (st.settings.push, st.settings.relay_wake) {
        (false, _) => "off (needs wake-ups on)".to_string(),
        (true, false) => "off".to_string(),
        (true, true) => format!(
            "on, {}{}",
            if st.relay_connected { "connected" } else { "not connected" },
            core.wake_relay().map(|r| format!(" ({r})")).unwrap_or_default()
        ),
    };
    println!("relay wake  {relay}");
    if st.refused > 0 {
        println!("refused     {} stale or repeated wakes", st.refused);
    }
    let devices = core.wakeable_devices();
    if !devices.is_empty() {
        println!("can wake:");
        for d in devices {
            println!("  {}  {}{}", d.label, d.peer_id, if d.through_relay { "  (through its relay)" } else { "" });
        }
    }
    Ok(())
}

fn ask(question: &str) -> bool {
    eprint!("{question} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    matches!(line.trim(), "y" | "Y" | "yes")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Sub, String> {
        parse(s.split_whitespace().map(String::from).collect())
    }

    #[test]
    fn parses_wake_commands() {
        assert_eq!(p(""), Ok(Sub::Status));
        assert_eq!(p("status"), Ok(Sub::Status));
        assert_eq!(p("off"), Ok(Sub::Push(false)));
        assert_eq!(p("on"), Ok(Sub::Push(true)));
        assert_eq!(p("relay on"), Ok(Sub::Relay { on: true, yes: false }));
        assert_eq!(p("relay off --yes"), Ok(Sub::Relay { on: false, yes: true }));
        assert!(p("relay").is_err());
        assert!(p("relay maybe").is_err());
        assert!(p("--bogus").is_err());
    }
}
