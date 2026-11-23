//! `bragi-tui linker …`: the desktop side of the desktop link, so a paired phone runs code mode
//! on this computer's projects (see `bragi_core::link` and `docs/desktop-link.md`).

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use bragi_core::link::server::{self, DEFAULT_PORT, OFFER_TTL, link_dir};
use bragi_core::link::{Confirm, LinkEvent, LinkServer, ServerOptions, Settings, ShellAsk};

pub const USAGE: &str = "\
usage: bragi-tui linker COMMAND
  pair [serve options]             share this computer with a phone: shows a QR code (and a code to
                                   type) for Bragi on the phone (Code > Pair with desktop), then keeps
                                   serving. Enter shows a new code, q quits
  serve [--pair] [--root DIR]... [--port N] [--no-weirdnet] [--any-interface] [--confirm-shell]
                                   serve paired phones without showing a code. --root adds a project
                                   folder to share (saved); the link listens on LAN and VPN addresses
                                   only (--any-interface: every address) on port 47480, and through
                                   the Weirdnet relay unless --no-weirdnet. Shell commands from a
                                   phone run as you; --confirm-shell asks here first (saved)
  devices                          paired phones
  revoke ID|NAME                   unpair a phone (a running linker refuses it at once)
  roots [add DIR | remove DIR]     the project folders phones may use; nothing else is reachable
  confirm on|off                   whether shell commands from a phone wait for a yes here";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sub {
    Serve { pair: bool, roots: Vec<PathBuf>, port: u16, weirdnet: bool, any: bool, confirm: Option<bool> },
    Devices,
    Revoke(String),
    Roots,
    RootAdd(PathBuf),
    RootRemove(PathBuf),
    Confirm(bool),
    Help,
}

pub fn parse(args: Vec<String>) -> Result<Sub, String> {
    let mut it = args.into_iter();
    let Some(first) = it.next() else { return Ok(Sub::Help) };
    let rest: Vec<String> = it.collect();
    match first.as_str() {
        "pair" | "serve" => {
            let (mut pair, mut roots, mut port, mut weirdnet, mut any, mut confirm) = (first == "pair", Vec::new(), DEFAULT_PORT, true, false, None);
            let mut it = rest.into_iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--pair" => pair = true,
                    "--root" => roots.push(PathBuf::from(it.next().ok_or("--root needs a folder")?)),
                    "--port" => port = it.next().ok_or("--port needs a number")?.parse().map_err(|_| "--port needs a number")?,
                    "--no-weirdnet" => weirdnet = false,
                    "--any-interface" => any = true,
                    "--confirm-shell" => confirm = Some(true),
                    "--no-confirm-shell" => confirm = Some(false),
                    other => return Err(format!("unknown option {other}\n{USAGE}")),
                }
            }
            Ok(Sub::Serve { pair, roots, port, weirdnet, any, confirm })
        }
        "devices" if rest.is_empty() => Ok(Sub::Devices),
        "revoke" if rest.len() == 1 => Ok(Sub::Revoke(rest[0].clone())),
        "roots" if rest.is_empty() => Ok(Sub::Roots),
        "roots" if rest.len() == 2 && rest[0] == "add" => Ok(Sub::RootAdd(crate::code::expand(&rest[1]))),
        "roots" if rest.len() == 2 && rest[0] == "remove" => Ok(Sub::RootRemove(crate::code::expand(&rest[1]))),
        "confirm" if rest.len() == 1 && (rest[0] == "on" || rest[0] == "off") => Ok(Sub::Confirm(rest[0] == "on")),
        "help" | "--help" | "-h" => Ok(Sub::Help),
        _ => Err(USAGE.to_string()),
    }
}

fn when(ms: i64) -> String {
    if ms <= 0 {
        return "never".into();
    }
    let ago = (bragi_core::history::now_ms() - ms) / 1000;
    match ago {
        s if s < 90 => "just now".into(),
        s if s < 90 * 60 => format!("{} min ago", s / 60),
        s if s < 36 * 3600 => format!("{} h ago", s / 3600),
        s => format!("{} days ago", s / 86400),
    }
}

pub fn run(home: &Path, sub: Sub) -> Result<(), String> {
    let dir = link_dir(home);
    match sub {
        Sub::Help => {
            println!("{USAGE}");
            Ok(())
        }
        Sub::Devices => {
            let d = server::devices(&dir);
            if d.is_empty() {
                println!("No paired phones. `bragi-tui linker pair` shows a code to pair one.");
            }
            for d in d {
                println!("{}  {}  paired {}, last seen {}", d.id, d.label, when(d.paired_ms), when(d.last_seen_ms));
            }
            Ok(())
        }
        Sub::Revoke(which) => {
            for d in server::revoke(&dir, &which).map_err(|e| e.message)? {
                println!("Unpaired {} ({}).", d.label, d.id);
            }
            Ok(())
        }
        Sub::Roots => {
            let s = Settings::load(&dir);
            if s.roots.is_empty() {
                println!("No project folders are shared. `bragi-tui linker roots add DIR` shares one.");
            }
            for (i, r) in s.roots.iter().enumerate() {
                println!("{i}  {}", r.display());
            }
            println!("Shell commands from a phone {}.", if s.confirm_shell { "wait for a yes here" } else { "run without asking" });
            Ok(())
        }
        Sub::RootAdd(p) => {
            let mut s = Settings::load(&dir);
            let new = s.add_root(&p).map_err(|e| e.message)?;
            s.save(&dir).map_err(|e| e.message)?;
            println!("{} {}", if new { "Sharing" } else { "Already sharing" }, p.display());
            Ok(())
        }
        Sub::RootRemove(p) => {
            let mut s = Settings::load(&dir);
            if !s.remove_root(&p) {
                return Err(format!("{} isn't shared", p.display()));
            }
            s.save(&dir).map_err(|e| e.message)?;
            println!("Stopped sharing {}", p.display());
            Ok(())
        }
        Sub::Confirm(on) => {
            let mut s = Settings::load(&dir);
            s.confirm_shell = on;
            s.save(&dir).map_err(|e| e.message)?;
            println!("Shell commands from a phone {}.", if on { "will wait for a yes here" } else { "will run without asking" });
            Ok(())
        }
        Sub::Serve { pair, roots, port, weirdnet, any, confirm } => serve(&dir, pair, roots, port, weirdnet, any, confirm),
    }
}

fn qr(text: &str) {
    if let Ok(qr) = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::L) {
        let art = qr
            .render::<qrcode::render::unicode::Dense1x2>()
            .quiet_zone(true)
            .dark_color(qrcode::render::unicode::Dense1x2::Light)
            .light_color(qrcode::render::unicode::Dense1x2::Dark)
            .build();
        println!("{art}");
    }
}

fn show_offer(server: &LinkServer) {
    let o = server.offer(OFFER_TTL);
    println!();
    qr(&o.url);
    println!("Scan with Bragi on your phone: Code > Pair with desktop. Good for {} minutes, once.", OFFER_TTL.as_secs() / 60);
    if let Some(short) = o.short {
        println!("Or type this code on the phone (same LAN or VPN): {short}");
    }
    println!("Desktop key: {}", server.peer_id());
}

enum Input {
    Line(String),
    Ask(ShellAsk, std::sync::mpsc::Sender<bool>),
    Event(LinkEvent),
}

#[allow(clippy::too_many_arguments)]
fn serve(dir: &Path, pair: bool, roots: Vec<PathBuf>, port: u16, weirdnet: bool, any: bool, confirm: Option<bool>) -> Result<(), String> {
    let mut s = Settings::load(dir);
    for r in &roots {
        s.add_root(r).map_err(|e| e.message)?;
    }
    if let Some(c) = confirm {
        s.confirm_shell = c;
    }
    if s.roots.is_empty() {
        let here = std::env::current_dir().map_err(|e| e.to_string())?;
        s.add_root(&here).map_err(|e| e.message)?;
        println!("No project folders were shared; sharing {} (add more with --root).", here.display());
    }
    s.save(dir).map_err(|e| e.message)?;

    let (tx, rx) = mpsc::channel::<Input>();
    let ask_tx = tx.clone();
    let confirm: Confirm = std::sync::Arc::new(move |ask: ShellAsk| {
        let (yes_tx, yes_rx) = std::sync::mpsc::channel();
        let _ = ask_tx.send(Input::Ask(ask, yes_tx));
        Box::pin(async move {
            // Answered from the terminal's thread; no answer is a no.
            let (t, r) = futures_channel();
            std::thread::spawn(move || {
                let _ = t.send(yes_rx.recv().unwrap_or(false));
            });
            r.await.unwrap_or(false)
        })
    });
    let mut opts = ServerOptions::new(dir, crate::session::label());
    opts.port = port;
    opts.weirdnet = weirdnet;
    opts.any_interface = any;
    let server = LinkServer::start(opts, Some(confirm)).map_err(|e| e.message)?;
    let direct = server.direct();
    println!("Bragi linker is serving {} folder{}:", s.roots.len(), if s.roots.len() == 1 { "" } else { "s" });
    for r in &s.roots {
        println!("  {}", r.display());
    }
    if direct.is_empty() {
        println!("Direct: no LAN or VPN address yet.");
    } else {
        println!("Direct: {}", direct.join("  "));
    }
    if weirdnet {
        // A relay reservation takes a moment; wait a little so the code carries the route.
        for _ in 0..50 {
            if server.on_weirdnet() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        println!(
            "Weirdnet: {}",
            if server.on_weirdnet() { "reachable through the relay" } else { "connecting to the relay (phones off the LAN may wait)" }
        );
    } else {
        println!("Weirdnet: off");
    }
    println!(
        "Shell commands from a phone {}.",
        if s.confirm_shell { "wait for a yes here" } else { "run as you without asking (--confirm-shell asks first)" }
    );
    if pair {
        show_offer(&server);
    }
    println!("Enter: a new pairing code · q: quit");

    let line_tx = tx.clone();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if line_tx.send(Input::Line(line)).is_err() {
                break;
            }
        }
        let _ = line_tx.send(Input::Line("q".into()));
    });
    let mut events = server.events();
    let ev_tx = tx;
    std::thread::spawn(move || {
        while let Ok(e) = events.blocking_recv() {
            if ev_tx.send(Input::Event(e)).is_err() {
                break;
            }
        }
    });

    let mut asking: std::collections::VecDeque<std::sync::mpsc::Sender<bool>> = Default::default();
    while let Ok(input) = rx.recv() {
        match input {
            Input::Ask(ask, answer) => {
                print!("\n{} wants to run in {}/{}:\n  $ {}\nRun it? [y/N] ", ask.device, ask.root.display(), ask.cwd, ask.command);
                let _ = std::io::stdout().flush();
                asking.push_back(answer);
            }
            Input::Line(l) => {
                let l = l.trim().to_lowercase();
                if let Some(answer) = asking.pop_front() {
                    let yes = l == "y" || l == "yes";
                    let _ = answer.send(yes);
                    println!("{}", if yes { "Running." } else { "Refused." });
                    continue;
                }
                match l.as_str() {
                    "q" | "quit" | "exit" => break,
                    _ => show_offer(&server),
                }
            }
            Input::Event(e) => match e {
                LinkEvent::Paired(d) => println!("Paired {} ({}). `bragi-tui linker revoke {}` unpairs it.", d.label, d.id, d.id),
                LinkEvent::Unpaired(d) => println!("{} unpaired itself.", d.label),
                LinkEvent::Refused(why) => println!("Refused: {why}"),
                LinkEvent::Shell { device, command } => println!("{device} ran: {command}"),
                LinkEvent::Wrote { device, files } => println!("{device} wrote {}", files.join(", ")),
            },
        }
    }
    server.stop();
    Ok(())
}

fn futures_channel() -> (tokio::sync::oneshot::Sender<bool>, tokio::sync::oneshot::Receiver<bool>) {
    tokio::sync::oneshot::channel()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        assert_eq!(parse(vec![]).unwrap(), Sub::Help);
        let s = parse(["pair", "--root", "/tmp", "--no-weirdnet", "--confirm-shell"].map(String::from).to_vec()).unwrap();
        assert_eq!(s, Sub::Serve { pair: true, roots: vec!["/tmp".into()], port: DEFAULT_PORT, weirdnet: false, any: false, confirm: Some(true) });
        assert_eq!(parse(["revoke", "d-1"].map(String::from).to_vec()).unwrap(), Sub::Revoke("d-1".into()));
        assert_eq!(parse(["confirm", "off"].map(String::from).to_vec()).unwrap(), Sub::Confirm(false));
        assert!(parse(["serve", "--bogus"].map(String::from).to_vec()).is_err());
    }

    #[test]
    fn roots_devices_and_confirm_are_saved() {
        let d = tempfile::tempdir().unwrap();
        let proj = d.path().join("p");
        std::fs::create_dir_all(&proj).unwrap();
        run(d.path(), Sub::RootAdd(proj.clone())).unwrap();
        run(d.path(), Sub::Confirm(true)).unwrap();
        let s = Settings::load(&link_dir(d.path()));
        assert_eq!(s.roots, vec![proj.canonicalize().unwrap()]);
        assert!(s.confirm_shell);
        run(d.path(), Sub::RootRemove(proj)).unwrap();
        assert!(Settings::load(&link_dir(d.path())).roots.is_empty());
        assert!(run(d.path(), Sub::Revoke("nobody".into())).is_err());
    }
}
