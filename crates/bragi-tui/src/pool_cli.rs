//! `bragi pool`: list, add, switch, rename and remove saved Ymir pools, for scripts and SSH
//! sessions. The same actions are in Settings in the terminal app.

use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

use bragi_core::{Core, PairState, PoolView};

use crate::session::{Launch, Session};

pub const USAGE: &str = "\
  bragi pool list
  bragi pool add [--yes]                 show a link code for another Ymir pool
  bragi pool switch ID|NAME              make that pool the active one
  bragi pool rename ID|NAME NEW-NAME
  bragi pool remove ID|NAME [--yes]      forget that pool on this device";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sub {
    List,
    Add { yes: bool },
    Switch(String),
    Rename { id: String, name: String },
    Remove { id: String, yes: bool },
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
    let mut it = words.into_iter();
    let sub = it.next().unwrap_or_else(|| "list".into());
    let rest: Vec<String> = it.collect();
    match sub.as_str() {
        "list" | "ls" if rest.is_empty() => Ok(Sub::List),
        "add" if rest.is_empty() => Ok(Sub::Add { yes }),
        "switch" | "use" => match rest.as_slice() {
            [id] => Ok(Sub::Switch(id.clone())),
            _ => Err("switch takes a pool id or name".into()),
        },
        "rename" => match rest.as_slice() {
            [id, name] => Ok(Sub::Rename { id: id.clone(), name: name.clone() }),
            _ => Err("rename takes a pool id or name and the new name".into()),
        },
        "remove" | "rm" | "delete" => match rest.as_slice() {
            [id] => Ok(Sub::Remove { id: id.clone(), yes }),
            _ => Err("remove takes a pool id or name".into()),
        },
        _ => Err(format!("pool needs list, add, switch, rename or remove\n{USAGE}")),
    }
}

pub fn run(launch: &Launch, sub: Sub) -> Result<(), String> {
    let s = Session::open(launch)?;
    match sub {
        Sub::List => list(&s.core),
        Sub::Add { yes } => add(&s.core, yes),
        Sub::Switch(id) => switch(&s.core, &id),
        Sub::Rename { id, name } => rename(&s.core, &id, &name),
        Sub::Remove { id, yes } => remove(&s.core, &id, yes),
    }
}

fn list(core: &Core) -> Result<(), String> {
    wait_ready(core);
    let pools = core.pools();
    if pools.is_empty() {
        println!("No Ymir pools yet. `bragi pool add` (or `bragi link`) links this device to one.");
        return Ok(());
    }
    for p in &pools {
        println!("{}", line(p));
    }
    Ok(())
}

fn line(p: &PoolView) -> String {
    let mark = if p.active { "*" } else { " " };
    let state = if p.linked { "linked" } else { "not linked" };
    let who = match (p.user.is_empty(), p.device.is_empty()) {
        (true, true) => String::new(),
        (false, true) => format!("  user {}", p.user),
        (_, false) => format!("  {} ({})", p.device, if p.user.is_empty() { "—" } else { p.user.as_str() }),
    };
    format!("{mark} {}  {}  {state}{who}", p.name, p.id)
}

fn add(core: &Core, yes: bool) -> Result<(), String> {
    wait_ready(core);
    let before = core.active_pool().map(|p| p.id);
    let rt = core.handle().clone();
    let link = rt.block_on(core.add_pool()).map_err(|e| e.message)?;
    println!("{link}");
    if let Ok(qr) = qrcode::QrCode::new(link.as_bytes()) {
        let art = qr
            .render::<qrcode::render::unicode::Dense1x2>()
            .quiet_zone(true)
            .dark_color(qrcode::render::unicode::Dense1x2::Light)
            .light_color(qrcode::render::unicode::Dense1x2::Dark)
            .build();
        eprintln!("{art}");
    }
    eprintln!("Open this code on a device of the other pool (Ymir: ymirctl pool add CODE; Bragi: Link a device).");
    loop {
        match core.state().pair {
            PairState::Confirm { sas } => {
                let ok = yes || ask(&format!("Does the other device show {sas}?"));
                core.pair_confirm(ok).map_err(|e| e.message)?;
                if !ok {
                    println!("Cancelled.");
                    return Ok(());
                }
            }
            PairState::Linked { .. } => {
                let now = core.active_pool();
                if before.as_deref() == now.as_ref().map(|p| p.id.as_str()) && before.is_some() {
                    println!("Cancelled.");
                } else if let Some(p) = now {
                    println!("Linked to {}.", p.name);
                } else {
                    println!("Linked.");
                }
                return Ok(());
            }
            PairState::Failed { message } => return Err(message),
            PairState::Unpaired => return Ok(()),
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn switch(core: &Core, id: &str) -> Result<(), String> {
    wait_ready(core);
    let rt = core.handle().clone();
    let p = rt.block_on(core.switch_pool(id)).map_err(|e| e.message)?;
    println!("Switched to {} ({}).", p.name, p.id);
    Ok(())
}

fn rename(core: &Core, id: &str, name: &str) -> Result<(), String> {
    wait_ready(core);
    let p = core.rename_pool(id, name).map_err(|e| e.message)?;
    println!("Renamed to {} ({}).", p.name, p.id);
    Ok(())
}

fn remove(core: &Core, id: &str, yes: bool) -> Result<(), String> {
    wait_ready(core);
    let id = core.resolve_pool(id).map_err(|e| e.message)?;
    let Some(p) = core.pools().into_iter().find(|p| p.id == id) else {
        return Err("no such pool".into());
    };
    if !yes && !ask(&format!("Forget {} ({}) on this device? Its history here is deleted.", p.name, p.id)) {
        println!("Kept.");
        return Ok(());
    }
    let rt = core.handle().clone();
    rt.block_on(core.remove_pool(&id)).map_err(|e| e.message)?;
    println!("Removed {}.", p.name);
    Ok(())
}

fn wait_ready(core: &Core) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && !matches!(core.state().pair, PairState::Linked { .. }) && core.has_identity() {
        std::thread::sleep(Duration::from_millis(50));
    }
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
        parse(s.split_whitespace().map(str::to_string).collect())
    }

    #[test]
    fn parse_subs() {
        assert_eq!(p("").unwrap(), Sub::List);
        assert_eq!(p("list").unwrap(), Sub::List);
        assert_eq!(p("add --yes").unwrap(), Sub::Add { yes: true });
        assert_eq!(p("switch Home").unwrap(), Sub::Switch("Home".into()));
        assert_eq!(p("rename Home Work").unwrap(), Sub::Rename { id: "Home".into(), name: "Work".into() });
        assert_eq!(p("remove Home -y").unwrap(), Sub::Remove { id: "Home".into(), yes: true });
        assert!(p("switch").is_err());
    }
}
