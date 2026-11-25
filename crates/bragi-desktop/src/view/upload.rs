//! The upload dialog on the Models screen (Ymir's model-upload plan, slice U4, desktop first):
//! pick a GGUF model or LoRA adapter on this device, see what it is before anything is sent, pick
//! the node (and an adapter's base), name it, give a license (optional), and send it with
//! progress and Cancel. Cancel, a dropped link or quitting Bragi leaves the parts on the node; the
//! screen lists unfinished uploads with Resume. The node's refusals (no space left after what it
//! keeps free, a name in use, an adapter for another base) and the llama.cpp restart note come
//! from `bragi-core`; this only asks and shows. Adding the upload to a rollout is not done here
//! yet: the result shows the `ymirctl` command for the inference node.

use std::path::PathBuf;

use bragi_core::{ModelsEvent, PendingUpload, UploadPreview, UploadSpec, UploadedView, human_bytes};

use super::*;

#[derive(Default)]
pub(super) struct UploadDialog {
    pub path: Option<PathBuf>,
    pub inspecting: bool,
    pub preview: Option<Result<UploadPreview, String>>,
    /// The model node, by peer.
    pub peer: Option<String>,
    /// For an adapter: the base on that node.
    pub base: Option<String>,
    /// Save it in the node's node.toml (on by default).
    pub persist: bool,
    /// The inference node of the rollout to add it to (by peer); the name is in `upload_rollout`.
    pub inference: Option<String>,
    /// Its weight there (0 to `MAX_WEIGHT`: a canary).
    pub weight: u32,
    pub run: Option<String>,
    /// Done, total, and the node's label.
    pub progress: Option<(u64, u64, String)>,
    pub result: Option<Result<UploadedView, String>>,
    pub ended: Option<String>,
    pub problem: Option<String>,
}

impl UploadDialog {
    pub fn sending(&self) -> bool {
        self.run.is_some() && self.result.is_none() && self.ended.is_none()
    }
}

impl BragiView {
    pub fn models_open_upload(&mut self, cx: &mut Context<Self>) {
        if self.models.owner_blocked() {
            return;
        }
        let peer = self.models.nodes.iter().flatten().find(|n| n.status.is_ready()).map(|n| n.node.peer.clone());
        self.models.upload = Some(UploadDialog { persist: true, peer, ..Default::default() });
        self.upload_name.update(cx, |i, cx| i.set_text("", cx));
        self.upload_license.update(cx, |i, cx| i.set_text("", cx));
        self.upload_rollout.update(cx, |i, cx| i.set_text("", cx));
        if let Some(d) = self.models.upload.as_mut() {
            d.inference = self.state.catalog.nodes.first().map(|n| n.id.clone());
        }
        self.models_pending = self.core.models_pending_uploads();
        cx.notify();
    }

    pub fn models_upload_pick_inference(&mut self, peer: &str, cx: &mut Context<Self>) {
        if let Some(d) = self.models.upload.as_mut().filter(|d| !d.sending()) {
            d.inference = Some(peer.to_string());
        }
        cx.notify();
    }

    pub fn models_upload_pick_weight(&mut self, w: u32, cx: &mut Context<Self>) {
        if let Some(d) = self.models.upload.as_mut().filter(|d| !d.sending()) {
            d.weight = w.min(bragi_core::models::ymir_rollouts::MAX_WEIGHT);
        }
        cx.notify();
    }

    /// The inference nodes (by peer) and their names, for the rollout section.
    pub(super) fn upload_inference_nodes(&self) -> Vec<(String, String)> {
        self.state
            .catalog
            .nodes
            .iter()
            .map(|n| {
                let label = self.models.nodes.iter().flatten().find(|m| m.node.peer == n.id).map(|m| m.node.label.clone());
                (n.id.clone(), label.unwrap_or_else(|| n.id.chars().take(12).collect()))
            })
            .collect()
    }

    pub fn models_close_upload(&mut self, cx: &mut Context<Self>) {
        if self.models.upload.as_ref().is_some_and(UploadDialog::sending) {
            return;
        }
        self.models.upload = None;
        self.models_pending = self.core.models_pending_uploads();
        cx.notify();
    }

    /// Opens the system file picker (never in the scenario, which sets the path itself).
    pub fn models_upload_browse(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(gpui::PathPromptOptions { files: true, directories: true, multiple: false, prompt: Some("Upload".into()) });
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(p) = paths.into_iter().next()
            {
                let _ = this.update(cx, |v, cx| v.models_upload_choose(p, cx));
            }
        })
        .detach();
    }

    /// Reads and hashes `path` here and shows what it is.
    pub fn models_upload_choose(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(d) = self.models.upload.as_mut() else { return };
        d.path = Some(path.clone());
        d.preview = None;
        d.inspecting = true;
        d.result = None;
        d.ended = None;
        d.problem = None;
        let core = self.core.clone();
        let job = self.core.handle().spawn(async move { core.models_inspect_upload(path).await });
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let r = job.await.unwrap_or_else(|e| Err(e.to_string()));
            let _ = this.update(cx, |v, cx| {
                if let Some(d) = v.models.upload.as_mut() {
                    d.inspecting = false;
                    if let Ok(p) = &r {
                        v.upload_name.update(cx, |i, cx| i.set_text(&p.suggested_id, cx));
                    }
                    d.preview = Some(r);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn models_upload_pick_node(&mut self, peer: &str, cx: &mut Context<Self>) {
        if let Some(d) = self.models.upload.as_mut().filter(|d| !d.sending()) {
            d.peer = Some(peer.to_string());
            d.base = None;
        }
        cx.notify();
    }

    pub fn models_upload_pick_base(&mut self, base: &str, cx: &mut Context<Self>) {
        if let Some(d) = self.models.upload.as_mut().filter(|d| !d.sending()) {
            d.base = Some(base.to_string());
        }
        cx.notify();
    }

    pub fn models_upload_toggle_persist(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.models.upload.as_mut().filter(|d| !d.sending()) {
            d.persist = !d.persist;
        }
        cx.notify();
    }

    /// The bases an adapter can run on at `peer`: the models served there.
    pub(super) fn upload_bases(&self, peer: &str) -> Vec<String> {
        self.models
            .nodes
            .iter()
            .flatten()
            .filter(|n| n.node.peer == peer)
            .flat_map(|n| n.models.iter())
            .filter(|m| m.base.is_none() && m.served_as.is_some())
            .filter_map(|m| m.served_as.clone())
            .collect()
    }

    pub fn models_upload_start(&mut self, cx: &mut Context<Self>) {
        let (name, license) = (self.upload_name.read(cx).text().trim().to_string(), self.upload_license.read(cx).text().trim().to_string());
        let rollout = self.upload_rollout.read(cx).text().trim().to_string();
        let Some(d) = self.models.upload.as_mut() else { return };
        if d.sending() {
            return;
        }
        let (Some(path), Some(Ok(preview))) = (d.path.clone(), d.preview.clone()) else {
            d.problem = Some("Pick a file first.".into());
            cx.notify();
            return;
        };
        let Some(peer) = d.peer.clone() else {
            d.problem = Some("Pick the node it goes to.".into());
            cx.notify();
            return;
        };
        if preview.is_adapter() && d.base.is_none() {
            d.problem = Some("Pick the base model this adapter runs on.".into());
            cx.notify();
            return;
        }
        let mut spec = UploadSpec::new(path, peer);
        spec.name = (!name.is_empty()).then_some(name);
        spec.license = (!license.is_empty()).then_some(license);
        spec.adapter_for = d.base.clone().filter(|_| preview.is_adapter());
        spec.persist = d.persist;
        if !rollout.is_empty() {
            let Some(inference) = d.inference.clone() else {
                d.problem = Some("Pick the inference node of the rollout.".into());
                cx.notify();
                return;
            };
            spec.rollout = Some(bragi_core::RolloutAsk { inference, name: rollout, weight: d.weight });
        }
        self.models_upload_send(spec, cx);
    }

    /// Sends `spec` (also a Resume: the node keeps the parts and the same files resume).
    pub(super) fn models_upload_send(&mut self, spec: UploadSpec, cx: &mut Context<Self>) {
        let mut run = match self.core.models_upload(spec) {
            Ok(r) => r,
            Err(e) => {
                if let Some(d) = self.models.upload.as_mut() {
                    d.problem = Some(e.message);
                }
                cx.notify();
                return;
            }
        };
        if let Some(d) = self.models.upload.as_mut() {
            d.run = Some(run.id.clone());
            d.progress = None;
            d.result = None;
            d.ended = None;
            d.problem = None;
        }
        self.models_pending = self.core.models_pending_uploads();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            while let Some(ev) = run.events.recv().await {
                let end = matches!(ev, ModelsEvent::Uploaded { .. } | ModelsEvent::Cancelled | ModelsEvent::Failed { .. });
                let alive = this.update(cx, |v, cx| {
                    v.on_upload(ev, cx);
                    cx.notify();
                });
                if alive.is_err() || end {
                    return;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn on_upload(&mut self, ev: ModelsEvent, cx: &mut Context<Self>) {
        let Some(d) = self.models.upload.as_mut() else { return };
        match ev {
            ModelsEvent::Progress { label, done, total, .. } => d.progress = Some((done, total, label)),
            ModelsEvent::Uploaded { result, .. } => {
                if let (Ok(_), Some(p)) = (&result, d.progress.as_mut()) {
                    p.0 = p.1;
                }
                d.result = Some(result);
                self.models_pending = self.core.models_pending_uploads();
                self.models_reload(cx);
            }
            ModelsEvent::Cancelled => {
                d.ended = Some("Cancelled. The node keeps what was sent for 7 days; Resume picks up from there.".into());
                self.models_pending = self.core.models_pending_uploads();
            }
            ModelsEvent::Failed { message } => {
                d.ended = Some(message);
                self.models_pending = self.core.models_pending_uploads();
            }
            _ => {}
        }
    }

    pub fn models_upload_cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = self.models.upload.as_ref().filter(|d| d.sending()).and_then(|d| d.run.clone()) {
            self.core.cancel(&run);
        }
        cx.notify();
    }

    /// Resumes an unfinished upload from the list: the dialog opens on it and sends it again.
    pub fn models_upload_resume(&mut self, p: PendingUpload, cx: &mut Context<Self>) {
        if self.models.upload.as_ref().is_some_and(UploadDialog::sending) {
            return;
        }
        self.models.upload = Some(UploadDialog {
            path: Some(p.spec.path.clone()),
            peer: Some(p.spec.peer.clone()),
            base: p.spec.adapter_for.clone(),
            persist: p.spec.persist,
            inference: p.spec.rollout.as_ref().map(|r| r.inference.clone()),
            weight: p.spec.rollout.as_ref().map_or(0, |r| r.weight),
            ..Default::default()
        });
        self.upload_rollout.update(cx, |i, cx| i.set_text(p.spec.rollout.as_ref().map_or("", |r| r.name.as_str()), cx));
        self.upload_name.update(cx, |i, cx| i.set_text(p.spec.name.as_deref().unwrap_or(""), cx));
        self.upload_license.update(cx, |i, cx| i.set_text(p.spec.license.as_deref().unwrap_or(""), cx));
        self.models_upload_send(p.spec, cx);
    }

    pub fn models_upload_forget(&mut self, p: &PendingUpload, cx: &mut Context<Self>) {
        self.core.models_forget_upload(&p.spec.path, &p.spec.peer);
        self.models_pending = self.core.models_pending_uploads();
        cx.notify();
    }

    // ---- drawing --------------------------------------------------------------------------

    /// "Unfinished uploads" on the Models screen.
    pub(super) fn upload_pending_list(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let t = self.theme;
        // The one being sent is in the dialog, not here.
        let sending = self.models.upload.as_ref().filter(|d| d.sending()).and_then(|d| d.path.clone().zip(d.peer.clone()));
        let shown: Vec<(usize, &PendingUpload)> = self
            .models_pending
            .iter()
            .enumerate()
            .filter(|(_, p)| !sending.as_ref().is_some_and(|(path, peer)| *path == p.spec.path && *peer == p.spec.peer))
            .collect();
        if shown.is_empty() {
            return None;
        }
        let mut col = div()
            .id("upload-pending")
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child("UNFINISHED UPLOADS"));
        for (i, p) in shown {
            let file = p.spec.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (a, b) = (p.clone(), p.clone());
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(8.))
                    .bg(rgb(t.surface))
                    .child(div().flex_1().text_size(px(13.)).child(format!("{file} to {} · {}", p.label, human_bytes(p.size))))
                    .child(
                        self.button(SharedString::from(format!("upload-resume-{i}")), "Resume", true)
                            .on_click(cx.listener(move |v, _, _, cx| v.models_upload_resume(a.clone(), cx))),
                    )
                    .child(
                        self.button(SharedString::from(format!("upload-forget-{i}")), "Forget", false)
                            .on_click(cx.listener(move |v, _, _, cx| v.models_upload_forget(&b, cx))),
                    ),
            );
        }
        Some(col.into_any_element())
    }

    fn upload_chip(&self, id: SharedString, label: String, on: bool) -> gpui::Stateful<gpui::Div> {
        let t = self.theme;
        div()
            .id(id)
            .px(px(10.))
            .py(px(4.))
            .rounded(px(12.))
            .text_size(px(12.))
            .border_1()
            .border_color(rgb(if on { t.accent } else { t.border }))
            .bg(rgb(if on { t.accent } else { t.background }))
            .text_color(rgb(if on { t.background } else { t.text_primary }))
            .cursor_pointer()
            .child(label)
    }

    /// The dialog over the window.
    pub(super) fn upload_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let t = self.theme;
        let d = self.models.upload.as_ref()?;
        let small = |s: String| div().text_size(px(12.)).text_color(rgb(t.text_secondary)).child(s);
        let field = |label: &str, input: Entity<TextInput>| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(div().text_size(px(12.)).child(label.to_string()))
                .child(div().px(px(10.)).py(px(6.)).rounded(px(6.)).bg(rgb(t.background)).text_size(px(13.)).child(input))
        };
        let mut card = div()
            .id("upload-dialog")
            .flex()
            .flex_col()
            .gap(px(10.))
            .w(px(560.))
            .max_h(px(660.))
            .overflow_y_scroll()
            .p(px(20.))
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(t.border))
            .bg(rgb(t.surface))
            .child(div().text_size(px(16.)).child("Upload a model"))
            .child(small("A GGUF model or LoRA adapter on this device goes to one of your model nodes, which serves it.".into()));
        // The file.
        // The file's name; its folder is in the rollout command once it is sent.
        let file = d
            .path
            .as_ref()
            .map_or("No file picked".to_string(), |p| p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned()));
        card = card.child(
            div().flex().flex_row().items_center().gap(px(8.)).child(div().id("upload-path").flex_1().text_size(px(13.)).child(file)).child(
                self.button("upload-browse", "Choose…", false).when(d.sending(), |b| b.opacity(0.5)).on_click(cx.listener(|v, _, _, cx| {
                    if !v.models.upload.as_ref().is_some_and(UploadDialog::sending) {
                        v.models_upload_browse(cx)
                    }
                })),
            ),
        );
        match &d.preview {
            None if d.inspecting => card = card.child(small("Reading and hashing it here…".into())),
            None => {}
            Some(Err(e)) => card = card.child(div().id("upload-preview-error").text_size(px(12.)).text_color(rgb(t.error)).child(e.clone())),
            Some(Ok(p)) => {
                let base = p.base_model.as_ref().map(|b| format!(" · made from {b}")).unwrap_or_default();
                card = card.child(
                    div()
                        .id("upload-preview")
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(px(8.))
                        .bg(rgb(t.background))
                        .text_size(px(13.))
                        .child(format!("{}{base}", p.summary())),
                );
            }
        }
        // Where it goes.
        let mut nodes = div().flex().flex_row().flex_wrap().items_center().gap(px(6.)).child(div().text_size(px(12.)).child("To"));
        for n in self.models.nodes.iter().flatten().filter(|n| n.status.is_ready()) {
            let peer = n.node.peer.clone();
            let on = d.peer.as_deref() == Some(peer.as_str());
            nodes = nodes.child(
                self.upload_chip(SharedString::from(format!("upload-node-{peer}")), n.node.label.clone(), on)
                    .on_click(cx.listener(move |v, _, _, cx| v.models_upload_pick_node(&peer, cx))),
            );
        }
        card = card.child(nodes);
        let adapter = matches!(&d.preview, Some(Ok(p)) if p.is_adapter());
        if adapter && let Some(peer) = &d.peer {
            let bases = self.upload_bases(peer);
            let mut row = div().flex().flex_row().flex_wrap().items_center().gap(px(6.)).child(div().text_size(px(12.)).child("Base"));
            if bases.is_empty() {
                row = row.child(small("That node serves no model to put it on.".into()));
            }
            for b in bases {
                let on = d.base.as_deref() == Some(b.as_str());
                let id = b.clone();
                row = row.child(
                    self.upload_chip(SharedString::from(format!("upload-base-{b}")), b, on)
                        .on_click(cx.listener(move |v, _, _, cx| v.models_upload_pick_base(&id, cx))),
                );
            }
            card = card.child(row);
        }
        card = card
            .child(field("Name", self.upload_name.clone()))
            .child(field("License (optional), such as mit or apache-2.0", self.upload_license.clone()));
        card = card.child(
            div()
                .id("upload-persist")
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.))
                .cursor_pointer()
                .child(div().text_size(px(13.)).child(if d.persist { "☑" } else { "☐" }))
                .child(div().text_size(px(13.)).child("Keep it after the node restarts (saved in its node.toml)"))
                .on_click(cx.listener(|v, _, _, cx| v.models_upload_toggle_persist(cx))),
        );
        // A rollout to add it to (optional): an inference node, the name, a canary weight.
        card = card.child(field("Add to a rollout (optional): its name, such as support", self.upload_rollout.clone()));
        let infs = self.upload_inference_nodes();
        let mut row = div().flex().flex_row().flex_wrap().items_center().gap(px(6.)).child(div().text_size(px(12.)).child("On"));
        if infs.is_empty() {
            row = row.child(small("no inference node answers right now".into()));
        }
        for (peer, label) in infs {
            let on = d.inference.as_deref() == Some(peer.as_str());
            row = row.child(
                self.upload_chip(SharedString::from(format!("upload-inf-{peer}")), label, on)
                    .on_click(cx.listener(move |v, _, _, cx| v.models_upload_pick_inference(&peer, cx))),
            );
        }
        row = row.child(div().text_size(px(12.)).child("Weight"));
        for w in [0u32, 1, 5, 10] {
            row = row.child(
                self.upload_chip(SharedString::from(format!("upload-weight-{w}")), w.to_string(), d.weight == w)
                    .on_click(cx.listener(move |v, _, _, cx| v.models_upload_pick_weight(w, cx))),
            );
        }
        card = card.child(row).child(small(
            "A new version starts as a canary (up to 10% of new conversations); raise it with `ymirctl models rollout weight --eval`.".into(),
        ));
        // Progress and the outcome.
        if let Some((done, total, label)) = &d.progress {
            let frac = if *total == 0 { 0.0 } else { (*done as f32 / *total as f32).clamp(0.0, 1.0) };
            card = card.child(
                div()
                    .id("upload-progress")
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(div().text_size(px(12.)).child(format!("{label}: {} of {}", human_bytes(*done), human_bytes(*total))))
                    .child(
                        div()
                            .h(px(6.))
                            .w_full()
                            .rounded(px(3.))
                            .bg(rgb(t.background))
                            .child(div().h_full().w(gpui::relative(frac)).rounded(px(3.)).bg(rgb(t.accent))),
                    ),
            );
        }
        if let Some(p) = &d.problem {
            card = card.child(div().text_size(px(12.)).text_color(rgb(t.warning)).child(p.clone()));
        }
        match &d.result {
            Some(Ok(u)) => {
                let lines = result_lines(u, &self.upload_node_label(d));
                let mut ok = div().id("upload-result").flex().flex_col().gap(px(4.)).text_size(px(13.));
                for l in lines {
                    ok = ok.child(div().child(l));
                }
                if let Some(n) = &u.adapter_note {
                    ok = ok.child(div().id("upload-adapter-note").text_size(px(12.)).text_color(rgb(t.warning)).child(n.clone()));
                }
                match &u.rollout {
                    Some(Ok(said)) => ok = ok.child(div().id("upload-rollout").text_size(px(13.)).child(said.clone())),
                    Some(Err(e)) => {
                        ok = ok.child(
                            div()
                                .id("upload-rollout-error")
                                .text_size(px(12.))
                                .text_color(rgb(t.warning))
                                .child(format!("Not added to the rollout: {e}")),
                        )
                    }
                    None => {}
                }
                if !u.rollout_commands.is_empty() {
                    ok = ok.child(small("To add it to a rollout with ymirctl:".into()));
                }
                for c in &u.rollout_commands {
                    ok = ok.child(div().px(px(8.)).py(px(4.)).rounded(px(6.)).bg(rgb(t.background)).text_size(px(12.)).child(c.clone()));
                }
                card = card.child(ok);
            }
            Some(Err(e)) => card = card.child(div().id("upload-refused").text_size(px(13.)).text_color(rgb(t.error)).child(e.clone())),
            None => {}
        }
        if let Some(e) = &d.ended {
            card = card.child(div().id("upload-ended").text_size(px(12.)).text_color(rgb(t.text_secondary)).child(e.clone()));
        }
        let mut buttons = div().flex().flex_row().gap(px(8.)).justify_end();
        if d.sending() {
            buttons = buttons.child(self.button("upload-cancel", "Cancel", false).on_click(cx.listener(|v, _, _, cx| v.models_upload_cancel(cx))));
        } else {
            let ready = matches!(&d.preview, Some(Ok(_))) && d.peer.is_some();
            buttons =
                buttons.child(self.button("upload-close", "Close", false).on_click(cx.listener(|v, _, _, cx| v.models_close_upload(cx)))).child(
                    self.button("upload-send", if d.ended.is_some() { "Resume" } else { "Upload" }, true)
                        .when(!ready, |b| b.opacity(0.5))
                        .on_click(cx.listener(|v, _, _, cx| v.models_upload_start(cx))),
                );
        }
        card = card.child(buttons);
        Some(div().absolute().inset_0().flex().items_center().justify_center().bg(gpui::rgba(0x00000099)).child(card).into_any_element())
    }

    fn upload_node_label(&self, d: &UploadDialog) -> String {
        d.peer
            .as_ref()
            .and_then(|p| self.models.nodes.iter().flatten().find(|n| &n.node.peer == p).map(|n| n.node.label.clone()))
            .unwrap_or_else(|| "The node".into())
    }
}

/// What the dialog says once the node has it: stored (resumed or not), already there, where it
/// was saved, and a register problem.
pub(super) fn result_lines(u: &UploadedView, node: &str) -> Vec<String> {
    let mut lines = vec![if u.already_there {
        format!("{node} already had {} with the same contents; nothing was sent.", u.id)
    } else if u.resumed >= u.size {
        format!("Stored {} ({}): the node already had all of it from the earlier try.", u.id, human_bytes(u.size))
    } else if u.resumed > 0 {
        format!("Stored {} ({}), resumed after {} already there.", u.id, human_bytes(u.size), human_bytes(u.resumed))
    } else {
        format!("Stored {} ({}) and serving it.", u.id, human_bytes(u.size))
    }];
    lines.extend(u.saved.clone());
    lines.extend(u.register_error.clone());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> UploadedView {
        UploadedView {
            id: "m".into(),
            revision: "r".into(),
            size: 2_000_000,
            resumed: 0,
            already_there: false,
            saved: Some("saved in gpu's node.toml".into()),
            register_error: None,
            adapter_note: None,
            rollout: None,
            rollout_commands: vec![],
        }
    }

    #[test]
    fn the_result_says_how_it_got_there() {
        assert_eq!(result_lines(&view(), "gpu"), ["Stored m (2.0 MB) and serving it.", "saved in gpu's node.toml"]);
        let resumed = UploadedView { resumed: 500_000, ..view() };
        assert!(result_lines(&resumed, "gpu")[0].contains("resumed after 500.0 kB"), "{:?}", result_lines(&resumed, "gpu"));
        let all = UploadedView { resumed: 2_000_000, ..view() };
        assert!(result_lines(&all, "gpu")[0].contains("already had all of it"));
        let there = UploadedView { already_there: true, saved: None, ..view() };
        assert_eq!(result_lines(&there, "gpu"), ["gpu already had m with the same contents; nothing was sent."]);
    }

    #[test]
    fn a_dialog_is_sending_until_it_ends() {
        let mut d = UploadDialog { run: Some("run".into()), ..Default::default() };
        assert!(d.sending());
        d.ended = Some("Cancelled.".into());
        assert!(!d.sending());
        assert!(!UploadDialog::default().sending());
    }
}
