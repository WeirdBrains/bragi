//! The U4 part of the desktop scenario: the upload dialog against the demo model nodes (Ymir's
//! model-upload plan). A model file and a LoRA adapter are made in a scratch dir; the upload is
//! cancelled part way and resumed from the list, the second demo node refuses for want of space
//! (it keeps more free than any disk has), and an adapter goes onto the uploaded model. Nothing
//! leaves the machine.

use bragi_core::demo_models::gguf;

use super::*;

fn e<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// A LoRA adapter GGUF header for a llama base.
fn lora(len: usize) -> Vec<u8> {
    let mut out = b"GGUF".to_vec();
    out.extend(3u32.to_le_bytes());
    out.extend(0u64.to_le_bytes());
    out.extend(3u64.to_le_bytes());
    for (k, v) in [("general.architecture", "llama"), ("general.type", "adapter"), ("adapter.type", "lora")] {
        out.extend((k.len() as u64).to_le_bytes());
        out.extend(k.as_bytes());
        out.extend(8u32.to_le_bytes());
        out.extend((v.len() as u64).to_le_bytes());
        out.extend(v.as_bytes());
    }
    out.resize(len, 0);
    out
}

fn peer_of(v: &BragiView, label: &str) -> Option<String> {
    v.models.nodes.iter().flatten().find(|n| n.node.label == label).map(|n| n.node.peer.clone())
}

fn result_ok(v: &BragiView) -> bool {
    v.models.upload.as_ref().is_some_and(|d| matches!(d.result, Some(Ok(_))))
}

pub(super) async fn run(this: &WeakEntity<BragiView>, cx: &mut gpui::AsyncApp, pause: Duration) -> Result<(), String> {
    let shot = async |cx: &mut gpui::AsyncApp, name: &str| {
        println!("bragi-scenario: screenshot {name}");
        cx.background_executor().timer(pause).await;
    };
    let dir = std::env::temp_dir().join(format!("bragi-scenario-upload-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(e)?;
    let (model, small, adapter) = (dir.join("my-finetune-Q8_0.gguf"), dir.join("notes-Q4_K_M.gguf"), dir.join("my-style-lora.gguf"));
    std::fs::write(&model, gguf(384 << 20)).map_err(e)?;
    std::fs::write(&small, gguf(1 << 20)).map_err(e)?;
    std::fs::write(&adapter, lora(2 << 20)).map_err(e)?;
    let out = steps(this, cx, &shot, &model, &small, &adapter).await;
    let _ = std::fs::remove_dir_all(&dir);
    out
}

async fn steps(
    this: &WeakEntity<BragiView>,
    cx: &mut gpui::AsyncApp,
    shot: &impl AsyncFn(&mut gpui::AsyncApp, &str),
    model: &Path,
    small: &Path,
    adapter: &Path,
) -> Result<(), String> {
    let (first, second) = this.read_with(cx, |v, _| (peer_of(v, "demo inference node"), peer_of(v, "demo inference node 2"))).map_err(e)?;
    let (first, second) = (first.ok_or("no first node")?, second.ok_or("no second node")?);
    let choose = |cx: &mut gpui::AsyncApp, p: &Path| {
        let p = p.to_path_buf();
        this.update(cx, move |v, cx| v.models_upload_choose(p, cx)).map_err(e)
    };
    let previewed = |v: &BragiView| v.models.upload.as_ref().is_some_and(|d| matches!(d.preview, Some(Ok(_))));

    // What it is, before anything is sent; the license is optional and given here.
    this.update(cx, |v, cx| {
        v.models_scroll.set_offset(gpui::point(px(0.), px(0.)));
        v.models_open_upload(cx)
    })
    .map_err(e)?;
    choose(cx, model)?;
    wait_for(this, cx, "the preview", previewed).await?;
    let f = first.clone();
    this.update(cx, move |v, cx| {
        v.models_upload_pick_node(&f, cx);
        v.upload_license.update(cx, |i, cx| i.set_text("apache-2.0", cx));
    })
    .map_err(e)?;
    shot(cx, "47-upload-preview").await;

    // Progress, then Cancel: the parts stay on the node.
    this.update(cx, |v, cx| v.models_upload_start(cx)).map_err(e)?;
    wait_for(this, cx, "upload progress", |v| v.models.upload.as_ref().and_then(|d| d.progress.as_ref()).is_some_and(|(d, t, _)| *d > 0 && d < t))
        .await?;
    shot(cx, "48-upload-progress").await;
    this.update(cx, |v, cx| v.models_upload_cancel(cx)).map_err(e)?;
    wait_for(this, cx, "the upload cancel", |v| v.models.upload.as_ref().is_some_and(|d| d.ended.is_some() || d.result.is_some())).await?;
    if this.read_with(cx, |v, _| result_ok(v)).map_err(e)? {
        return Err("the upload finished before Cancel".into());
    }
    this.update(cx, |v, cx| v.models_close_upload(cx)).map_err(e)?;
    wait_for(this, cx, "the unfinished upload", |v| v.models_pending.len() == 1).await?;
    shot(cx, "49-upload-unfinished").await;

    // Resume from the list: it picks up where the node stopped.
    let pending = this.read_with(cx, |v, _| v.models_pending[0].clone()).map_err(e)?;
    this.update(cx, move |v, cx| v.models_upload_resume(pending, cx)).map_err(e)?;
    wait_for(this, cx, "the resumed upload", |v| v.models.upload.as_ref().is_some_and(|d| d.result.is_some() || d.ended.is_some())).await?;
    let (resumed, err) = this
        .read_with(cx, |v, _| match v.models.upload.as_ref().and_then(|d| d.result.clone()) {
            Some(Ok(u)) => (u.resumed, None),
            Some(Err(m)) => (0, Some(m)),
            None => (0, v.models.upload.as_ref().and_then(|d| d.ended.clone())),
        })
        .map_err(e)?;
    if let Some(m) = err {
        return Err(format!("the resume failed: {m}"));
    }
    if resumed == 0 {
        return Err("the resume sent everything again".into());
    }
    if !this.read_with(cx, |v, _| v.models_pending.is_empty()).map_err(e)? {
        return Err("a finished upload is still listed".into());
    }
    shot(cx, "50-upload-resumed").await;

    // The second node keeps more free than it has: it refuses, and says how to change that.
    this.update(cx, |v, cx| v.models_close_upload(cx)).map_err(e)?;
    this.update(cx, |v, cx| v.models_open_upload(cx)).map_err(e)?;
    choose(cx, small)?;
    wait_for(this, cx, "the small preview", previewed).await?;
    let s = second.clone();
    this.update(cx, move |v, cx| {
        v.models_upload_pick_node(&s, cx);
        v.models_upload_start(cx)
    })
    .map_err(e)?;
    wait_for(this, cx, "the refusal", |v| v.models.upload.as_ref().is_some_and(|d| d.result.is_some() || d.ended.is_some())).await?;
    let refusal = this.read_with(cx, |v, _| v.models.upload.as_ref().and_then(|d| d.result.clone())).map_err(e)?;
    match refusal {
        Some(Err(m)) if m.contains("upload_keep_free") => {}
        other => return Err(format!("no free-space refusal: {other:?}")),
    }
    shot(cx, "51-upload-no-space").await;

    // An adapter goes onto the uploaded model on the first node.
    this.update(cx, |v, cx| v.models_close_upload(cx)).map_err(e)?;
    let f = first.clone();
    wait_for(this, cx, "the uploaded base", move |v| !v.upload_bases(&f).is_empty()).await?;
    this.update(cx, |v, cx| v.models_open_upload(cx)).map_err(e)?;
    choose(cx, adapter)?;
    wait_for(this, cx, "the adapter preview", previewed).await?;
    let f = first.clone();
    this.update(cx, move |v, cx| {
        v.models_upload_pick_node(&f, cx);
        if let Some(b) = v.upload_bases(&f).into_iter().find(|b| b.contains("finetune")) {
            v.models_upload_pick_base(&b, cx);
        }
        v.models_upload_start(cx)
    })
    .map_err(e)?;
    wait_for(this, cx, "the adapter upload", |v| v.models.upload.as_ref().is_some_and(|d| d.result.is_some() || d.ended.is_some())).await?;
    let got = this.read_with(cx, |v, _| v.models.upload.as_ref().and_then(|d| d.result.clone())).map_err(e)?;
    let Some(Ok(u)) = got else { return Err(format!("the adapter upload: {got:?}")) };
    if u.adapter_note.is_none() {
        // The demo's engine takes adapters live; a llama.cpp node takes them at the next start and
        // Bragi says so. That note comes from bragi-core (tested there); it is shown here as the
        // view would get it.
        println!("bragi-scenario: the llama.cpp restart note is a seeded view state");
        this.update(cx, |v, cx| {
            if let Some(d) = v.models.upload.as_mut()
                && let Some(Ok(u)) = d.result.as_mut()
            {
                let base = d.base.clone().unwrap_or_default();
                u.adapter_note = Some(format!(
                    "`{}` is added to `{base}` on demo inference node: llama.cpp loads adapters when it starts, so it is used from `{base}`'s next start; while `{base}` runs, the first request for `{}` restarts it",
                    u.id, u.id
                ));
            }
            cx.notify();
        })
        .map_err(e)?;
    }
    shot(cx, "52-upload-adapter").await;
    this.update(cx, |v, cx| v.models_close_upload(cx)).map_err(e)?;

    // Into a rollout: the next version on an inference node, at a canary weight.
    this.update(cx, |v, cx| v.models_open_upload(cx)).map_err(e)?;
    choose(cx, small)?;
    wait_for(this, cx, "the rollout preview", previewed).await?;
    if this.read_with(cx, |v, _| v.upload_inference_nodes().is_empty()).map_err(e)? {
        return Err("no inference node for the rollout".into());
    }
    let f = first.clone();
    this.update(cx, move |v, cx| {
        v.models_upload_pick_node(&f, cx);
        v.upload_rollout.update(cx, |i, cx| i.set_text("support", cx));
        v.models_upload_pick_weight(5, cx);
    })
    .map_err(e)?;
    shot(cx, "53-upload-rollout-ask").await;
    this.update(cx, |v, cx| v.models_upload_start(cx)).map_err(e)?;
    wait_for(this, cx, "the rollout upload", |v| v.models.upload.as_ref().is_some_and(|d| d.result.is_some() || d.ended.is_some())).await?;
    let got = this.read_with(cx, |v, _| v.models.upload.as_ref().and_then(|d| d.result.clone())).map_err(e)?;
    match got {
        Some(Ok(u)) if u.rollout.as_ref().is_some_and(|r| r.as_ref().is_ok_and(|s| s.contains("support@1"))) => {}
        other => return Err(format!("the rollout was not added: {other:?}")),
    }
    shot(cx, "54-upload-rollout-added").await;
    this.update(cx, |v, cx| v.models_close_upload(cx)).map_err(e)?;

    // Copied on to another node: the second keeps more free than it has, so it is left out first.
    this.update(cx, |v, cx| v.models_open_upload(cx)).map_err(e)?;
    choose(cx, small)?;
    wait_for(this, cx, "the copy preview", previewed).await?;
    let (f, s2) = (first.clone(), second.clone());
    this.update(cx, move |v, cx| {
        v.models_upload_pick_node(&f, cx);
        v.models_upload_toggle_copy(&s2, cx);
    })
    .map_err(e)?;
    shot(cx, "55-upload-copy-ask").await;
    this.update(cx, |v, cx| v.models_upload_start(cx)).map_err(e)?;
    wait_for(this, cx, "the copy upload", |v| v.models.upload.as_ref().is_some_and(|d| d.result.is_some() || d.ended.is_some())).await?;
    let got = this.read_with(cx, |v, _| v.models.upload.as_ref().and_then(|d| d.result.clone())).map_err(e)?;
    match got {
        Some(Ok(u)) if u.copies.len() == 1 && u.copies[0].1.as_ref().is_err_and(|m| m.starts_with("left out:")) => {}
        other => return Err(format!("the copy was not left out: {other:?}")),
    }
    shot(cx, "56-upload-copy-left-out").await;
    this.update(cx, |v, cx| v.models_close_upload(cx)).map_err(e)?;
    println!("bragi-scenario: uploads");
    Ok(())
}
