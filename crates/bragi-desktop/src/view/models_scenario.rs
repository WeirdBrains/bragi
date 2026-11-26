//! The LF-30 part of the desktop scenario: the Models screen against the demo model nodes (two
//! nodes of one owner behind the owner-only models/1) and a mock Hub on a loopback port. Nothing
//! leaves the machine, and no real node is touched.

use bragi_core::NodeStatus;

use super::models::Pick;
use super::*;

fn e<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// The fits that came from nodes that answered (the others carry only their status).
fn ready_fits(v: &BragiView) -> usize {
    v.models.fits.values().flatten().filter(|f| f.status.is_ready()).count()
}

pub(super) async fn run(this: &WeakEntity<BragiView>, cx: &mut gpui::AsyncApp, pause: Duration) -> Result<(), String> {
    let shot = async |cx: &mut gpui::AsyncApp, name: &str| {
        println!("bragi-scenario: screenshot {name}");
        cx.background_executor().timer(pause).await;
    };
    // The screen scrolls; a shot of a section scrolls it into view first (the offsets are of the
    // 1100x720 virtual window).
    let scroll_to = |cx: &mut gpui::AsyncApp, y: f32| {
        this.update(cx, move |v, cx| {
            v.models_scroll.set_offset(gpui::point(px(0.), px(-y)));
            cx.notify();
        })
        .map_err(e)
    };
    let query = |cx: &mut gpui::AsyncApp, text: &str| {
        let text = text.to_string();
        this.update(cx, move |v, cx| v.models_query.update(cx, |i, cx| i.set_text(&text, cx))).map_err(e)
    };

    // Open the screen: both demo nodes take model management from this linked device; the demo's
    // other laptop is a device that is not a model node, and says so.
    this.update(cx, |v, cx| {
        if v.settings_open {
            v.toggle_settings(cx);
        }
        v.toggle_models(cx);
    })
    .map_err(e)?;
    wait_for(this, cx, "the nodes", |v| v.models.nodes.as_ref().is_some_and(|n| n.iter().filter(|n| n.status == NodeStatus::Ready).count() == 2))
        .await?;
    if this.read_with(cx, |v, _| v.models.owner_blocked()).map_err(e)? {
        return Err("a linked device was not admitted by the nodes".into());
    }
    shot(cx, "40-models-nodes").await;

    // Search: a fit badge per node.
    query(cx, "qwen")?;
    this.update(cx, |v, cx| v.models_search(cx)).map_err(e)?;
    wait_for(this, cx, "the search", |v| !v.models.searching && v.models.hits.len() == 2 && ready_fits(v) == 4).await?;
    let verdicts = this
        .read_with(cx, |v, _| {
            let mut out: Vec<(String, String, bragi_core::Verdict)> = Vec::new();
            for (repo, fits) in &v.models.fits {
                for f in fits.iter().filter(|f| f.status.is_ready()) {
                    out.push((repo.clone(), f.node.label.clone(), f.verdict()));
                }
            }
            out
        })
        .map_err(e)?;
    let fits = verdicts.iter().filter(|(_, _, v)| *v == bragi_core::Verdict::Fits).count();
    let too_big = verdicts.iter().filter(|(_, _, v)| *v == bragi_core::Verdict::TooBig).count();
    if (fits, too_big) != (3, 1) {
        return Err(format!("fit badges: {verdicts:?}"));
    }
    scroll_to(cx, 230.)?;
    cx.background_executor().timer(Duration::from_millis(300)).await;
    shot(cx, "41-models-search").await;

    // Pull the tiny repo to all nodes, watch progress, cancel.
    query(cx, "tiny")?;
    this.update(cx, |v, cx| {
        v.models_pick(Pick::All, cx);
        v.models_search(cx)
    })
    .map_err(e)?;
    wait_for(this, cx, "the tiny search", |v| !v.models.searching && v.models.hits.len() == 1 && ready_fits(v) == 2).await?;
    this.update(cx, |v, cx| v.models_pull("demo/Tiny-Chat-GGUF", cx)).map_err(e)?;
    wait_for(this, cx, "progress", |v| v.models.pull.as_ref().is_some_and(|p| p.progress.values().any(|(_, d, t)| *d > 0 && d < t))).await?;
    cx.background_executor().timer(Duration::from_millis(700)).await;
    scroll_to(cx, 160.)?;
    cx.background_executor().timer(Duration::from_millis(300)).await;
    shot(cx, "42-models-progress").await;
    this.update(cx, |v, cx| v.models_cancel_pull(cx)).map_err(e)?;
    wait_for(this, cx, "the cancel", |v| v.models.pull.as_ref().is_some_and(|p| p.finished)).await?;
    if !this.read_with(cx, |v, _| v.models.pull.as_ref().and_then(|p| p.ended.clone()).is_some_and(|m| m.starts_with("Cancelled"))).map_err(e)? {
        return Err("the pull did not report the cancel".into());
    }
    shot(cx, "43-models-cancelled").await;
    this.update(cx, |v, cx| v.models_dismiss_pull(cx)).map_err(e)?;

    // Auto picks the node where the biggest fits, and says why; then it is listed and serving.
    this.update(cx, |v, cx| {
        v.models_pick(Pick::Auto, cx);
        v.models_pull("demo/Tiny-Chat-GGUF", cx)
    })
    .map_err(e)?;
    wait_for(this, cx, "the pull", |v| {
        v.models.pull.as_ref().is_some_and(|p| p.finished && p.results.iter().any(|r| r.contains("downloaded and serving")))
    })
    .await?;
    if !this
        .read_with(cx, |v, _| v.models.pull.as_ref().and_then(|p| p.chosen.clone()).is_some_and(|c| c.contains("can hold the biggest one")))
        .map_err(e)?
    {
        return Err("auto did not say why".into());
    }
    wait_for(this, cx, "the list", |v| {
        v.models.nodes.iter().flatten().any(|n| n.models.iter().any(|m| m.id == "Tiny-Chat-Q8_0" && m.served_as.is_some()))
    })
    .await?;
    scroll_to(cx, 0.)?;
    cx.background_executor().timer(Duration::from_millis(300)).await;
    shot(cx, "44-models-pulled").await;

    // Remove asks first; Cancel keeps it, Remove deletes it.
    let (peer, id) = this
        .read_with(cx, |v, _| v.models.nodes.iter().flatten().find_map(|n| n.models.first().map(|m| (n.node.peer.clone(), m.id.clone()))))
        .map_err(e)?
        .ok_or("nothing to remove")?;
    this.update(cx, |v, cx| v.models_ask_remove(&peer, &id, cx)).map_err(e)?;
    shot(cx, "45-models-remove-confirm").await;
    this.update(cx, |v, cx| v.models_cancel_remove(cx)).map_err(e)?;
    if this.read_with(cx, |v, _| v.models.nodes.iter().flatten().all(|n| n.models.is_empty())).map_err(e)? {
        return Err("cancelling the confirm removed the model".into());
    }
    this.update(cx, |v, cx| {
        v.models_ask_remove(&peer, &id, cx);
        v.models_confirm_remove(cx)
    })
    .map_err(e)?;
    wait_for(this, cx, "the removal", |v| {
        v.models.nodes.iter().flatten().all(|n| n.models.is_empty()) && v.models.notice.as_deref().is_some_and(|n| n.contains("freed"))
    })
    .await?;
    shot(cx, "46-models-removed").await;
    super::upload_scenario::run(this, cx, pause).await?;
    // The Training toggle: the demo nodes' finished run, and the rollout the upload made.
    scroll_to(cx, 0.)?;
    this.update(cx, |v, cx| v.toggle_training(true, cx)).map_err(e)?;
    wait_for(this, cx, "the training view", |v| v.training.overview.as_ref().is_some_and(|o| o.ready().count() == 2 && !o.rollouts.is_empty()))
        .await?;
    let support = this
        .read_with(cx, |v, _| v.training.overview.as_ref().is_some_and(|o| o.rollouts.iter().any(|r| r.name == "support" && !r.versions.is_empty())))
        .map_err(e)?;
    if !support {
        return Err("the training view does not show the upload's rollout".into());
    }
    shot(cx, "60-training").await;
    this.update(cx, |v, cx| v.toggle_training(false, cx)).map_err(e)?;
    this.update(cx, |v, cx| v.toggle_models(cx)).map_err(e)?;
    println!("bragi-scenario: models");
    Ok(())
}
