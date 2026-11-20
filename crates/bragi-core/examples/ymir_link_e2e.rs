//! The Bragi side of `scripts/ymir-link-e2e.sh`: a headless Bragi core that is linked to a real
//! `ymird` pool and then admits another real Ymir node, both with throwaway state. Not shipped;
//! it prints one `KEY value` line per step for the script to check.
//!
//! `ymir_link_e2e DIR YMIR_ADDR WORK`: `DIR` is Bragi's data folder, `YMIR_ADDR` the node to dial
//! (`/ip4/127.0.0.1/tcp/…/p2p/…`), `WORK` a folder shared with the script for hand-offs.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use bragi_core::{ChatEvent, Config, Core, PairState, Transport};

async fn until<T>(what: &str, secs: u64, mut f: impl FnMut() -> Option<T>) -> Result<T> {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f() {
            return Ok(v);
        }
        if Instant::now() > end {
            bail!("timed out waiting for {what}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn read_file(path: &Path, what: &str, secs: u64) -> Result<String> {
    until(what, secs, || std::fs::read_to_string(path).ok().filter(|s| !s.trim().is_empty())).await.map(|s| s.trim().to_string())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let dir: PathBuf = args.next().context("DIR")?.into();
    let ymir = args.next().context("YMIR_ADDR")?;
    let work: PathBuf = args.next().context("WORK")?.into();

    let mut cfg = Config::new(&dir, "bragi-e2e");
    cfg.transport = Transport::Internet;
    cfg.peers = vec![ymir];
    let core = Core::start(cfg)?;

    // Direction 1: Bragi shows the code, `ymirctl pool add` opens it.
    let link = core.pair_offer().await?;
    println!("BRAGI_CODE_SCHEME {}", link.split("link/").next().unwrap_or(""));
    std::fs::write(work.join("bragi.link"), &link)?;
    let sas = until("the code check", 120, || match core.state().pair {
        PairState::Confirm { sas } => Some(sas),
        PairState::Failed { message } => panic!("pairing failed: {message}"),
        _ => None,
    })
    .await?;
    println!("BRAGI_SAS {sas}");
    std::fs::write(work.join("bragi.sas"), &sas)?;
    core.pair_confirm(true)?;
    until("linked and online", 60, || matches!(core.state().pair, PairState::Linked { .. }).then(|| core.state().online).filter(|o| *o)).await?;

    // Bragi lists the pool's models and gets a reply through the pool.
    let models = until("the pool's models", 90, || {
        let m: Vec<String> = core.state().catalog.models.iter().map(|m| m.id.clone()).collect();
        (!m.is_empty()).then_some(m)
    })
    .await?;
    println!("BRAGI_MODELS {}", models.join(","));
    let mut chat = core.send(None, "ping from bragi", Some(models[0].clone()))?;
    let mut text = String::new();
    while let Some(ev) = tokio::time::timeout(Duration::from_secs(60), chat.events.recv()).await.context("the answer stalled")? {
        match ev {
            ChatEvent::Delta { offset, text: t } => {
                text = text.chars().take(offset as usize).collect::<String>() + &t;
            }
            ChatEvent::Done { .. } => break,
            ChatEvent::Error { .. } => bail!("chat error: {ev:?}"),
            _ => {}
        }
    }
    println!("BRAGI_REPLY {text}");
    std::fs::write(work.join("bragi.done1"), "ok")?;

    // Direction 2: a new Ymir node shows `ymirctl pool link`'s code, and Bragi (already linked) opens it.
    let code = read_file(&work.join("ymir2.link"), "the second node's code", 180).await?;
    let check = core.pair_accept(&code).await?;
    println!("BRAGI_ACCEPT_SAS {}", check.sas);
    std::fs::write(work.join("bragi.accept.sas"), &check.sas)?;
    // The script compares the check code with what ymirctl shows, then lets us confirm.
    read_file(&work.join("confirm2"), "the script's go-ahead", 120).await?;
    core.pair_accept_confirm(true).await?;
    println!("BRAGI_ADDED {}", check.device);
    std::fs::write(work.join("bragi.done2"), "ok")?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    core.shutdown();
    Ok(())
}
