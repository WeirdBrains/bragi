//! The Models screen (LF-30): search the Hugging Face Hub with a fit badge per node, pull to a node,
//! the best one (auto) or all of them with progress and cancel, see what each node has, and remove
//! behind a confirm. Every action is one `Core` call; this only asks and shows, the same flow as the
//! Flutter app. The nodes admit only devices linked to the owner: one that refuses this device shows
//! "needs owner device", and when every node does the controls are off behind a banner.

use std::collections::BTreeMap;

use bragi_core::{HubHit, ModelsEvent, NodeFit, NodeModels, NodeStatus, PullSpec, PullTarget, StoredView, Verdict, human_bytes};

use super::*;

/// A pull as the screen shows it.
#[derive(Default)]
pub(super) struct PullView {
    pub repo: String,
    pub run: String,
    pub chosen: Option<String>,
    /// By peer: label, done, total.
    pub progress: BTreeMap<String, (String, u64, u64)>,
    pub results: Vec<String>,
    pub finished: bool,
    pub ended: Option<String>,
}

/// A remove waiting for its confirm.
pub(super) struct RemoveAsk {
    pub peer: String,
    pub node: String,
    pub model: StoredView,
}

/// Where the next pull goes, as the picker has it.
#[derive(Clone, PartialEq, Eq, Default)]
pub(super) enum Pick {
    #[default]
    Auto,
    All,
    Node(String),
}

#[derive(Default)]
pub(super) struct Models {
    pub nodes: Option<Vec<NodeModels>>,
    pub error: Option<String>,
    pub hits: Vec<HubHit>,
    pub fits: HashMap<String, Vec<NodeFit>>,
    pub searching: bool,
    pub search_run: Option<String>,
    pub notice: Option<String>,
    pub pick: Pick,
    pub serve: bool,
    /// The quantization chosen per repo (`None`: the best that fits).
    pub quant: HashMap<String, String>,
    pub pull: Option<PullView>,
    pub remove: Option<RemoveAsk>,
}

fn ready(n: &NodeModels) -> bool {
    n.status.is_ready()
}

fn status_text(s: &NodeStatus) -> &'static str {
    match s {
        NodeStatus::Ready => "Ready",
        NodeStatus::NotModelNode => "Not a model node",
        NodeStatus::NeedsOwnerDevice(_) => "Needs owner device",
        _ => "Unreachable",
    }
}

impl Models {
    /// Every node refused this device.
    pub fn owner_blocked(&self) -> bool {
        self.nodes.as_ref().is_some_and(|n| !n.is_empty() && n.iter().all(|x| matches!(x.status, NodeStatus::NeedsOwnerDevice(_))))
    }

    fn ready_nodes(&self) -> impl Iterator<Item = &NodeModels> {
        self.nodes.iter().flatten().filter(|n| ready(n))
    }
}

impl BragiView {
    pub fn toggle_models(&mut self, cx: &mut Context<Self>) {
        self.models_open = !self.models_open;
        self.settings_open = false;
        if self.models_open {
            self.models_reload(cx);
        }
        cx.notify();
    }

    /// Asks every device what it has.
    pub fn models_reload(&mut self, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let job = self.core.handle().spawn(async move { core.models_overview().await });
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let r = job.await;
            let _ = this.update(cx, |v, cx| {
                match r {
                    Ok(Ok(nodes)) => {
                        if let Pick::Node(p) = &v.models.pick
                            && !nodes.iter().any(|n| &n.node.peer == p && ready(n))
                        {
                            v.models.pick = Pick::Auto;
                        }
                        v.models.nodes = Some(nodes);
                        v.models.error = None;
                    }
                    Ok(Err(e)) => v.models.error = Some(e.message),
                    Err(e) => v.models.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn models_search(&mut self, cx: &mut Context<Self>) {
        let q = self.models_query.read(cx).text().trim().to_string();
        if q.is_empty() || self.models.owner_blocked() {
            return;
        }
        if let Some(old) = self.models.search_run.take() {
            self.core.cancel(&old);
        }
        let mut run = match self.core.models_search(&q) {
            Ok(r) => r,
            Err(e) => {
                self.models.notice = Some(e.message);
                cx.notify();
                return;
            }
        };
        let m = &mut self.models;
        m.hits.clear();
        m.fits.clear();
        m.notice = None;
        m.searching = true;
        m.search_run = Some(run.id.clone());
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            while let Some(ev) = run.events.recv().await {
                let end = matches!(ev, ModelsEvent::SearchDone { .. } | ModelsEvent::Cancelled | ModelsEvent::Failed { .. });
                let alive = this.update(cx, |v, cx| {
                    // A newer search replaced this one.
                    if v.models.search_run.as_deref() != Some(run.id.as_str()) {
                        return;
                    }
                    match ev {
                        ModelsEvent::Hits(h) => v.models.hits = h,
                        ModelsEvent::Fit { repo, fit } => v.models.fits.entry(repo).or_default().push(fit),
                        ModelsEvent::SearchDone { error } => {
                            v.models.searching = false;
                            v.models.notice = error;
                        }
                        ModelsEvent::Failed { message } => {
                            v.models.searching = false;
                            v.models.notice = Some(message);
                        }
                        _ => v.models.searching = false,
                    }
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

    pub(super) fn models_pick(&mut self, pick: Pick, cx: &mut Context<Self>) {
        self.models.pick = pick;
        cx.notify();
    }

    pub fn models_toggle_serve(&mut self, cx: &mut Context<Self>) {
        self.models.serve = !self.models.serve;
        cx.notify();
    }

    /// Steps through the repo's quantizations (as the nodes rank them), then back to "best that fits".
    pub fn models_cycle_quant(&mut self, repo: &str, cx: &mut Context<Self>) {
        let mut ids: Vec<String> = Vec::new();
        for f in self.models.fits.get(repo).into_iter().flatten() {
            for q in &f.quants {
                if !ids.contains(&q.id) {
                    ids.push(q.id.clone());
                }
            }
        }
        let next = match self.models.quant.get(repo) {
            None => ids.first().cloned(),
            Some(cur) => ids.iter().position(|i| i == cur).and_then(|i| ids.get(i + 1).cloned()),
        };
        match next {
            Some(q) => self.models.quant.insert(repo.to_string(), q),
            None => self.models.quant.remove(repo),
        };
        cx.notify();
    }

    pub fn models_pull(&mut self, repo: &str, cx: &mut Context<Self>) {
        if self.models.owner_blocked() || self.models.pull.as_ref().is_some_and(|p| !p.finished) {
            return;
        }
        let target = match &self.models.pick {
            Pick::Auto => PullTarget::Auto,
            Pick::All => PullTarget::All,
            Pick::Node(p) => PullTarget::Node(p.clone()),
        };
        let mut spec = PullSpec::new(repo, target);
        spec.model = self.models.quant.get(repo).cloned();
        spec.serve = self.models.serve;
        let mut run = match self.core.models_pull(spec) {
            Ok(r) => r,
            Err(e) => {
                self.models.notice = Some(e.message);
                cx.notify();
                return;
            }
        };
        self.models.pull = Some(PullView { repo: repo.to_string(), run: run.id.clone(), ..Default::default() });
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            while let Some(ev) = run.events.recv().await {
                let end = matches!(ev, ModelsEvent::Done | ModelsEvent::Cancelled | ModelsEvent::Failed { .. });
                let alive = this.update(cx, |v, cx| {
                    v.on_pull(ev, cx);
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

    fn on_pull(&mut self, ev: ModelsEvent, cx: &mut Context<Self>) {
        let Some(p) = self.models.pull.as_mut() else { return };
        match ev {
            ModelsEvent::Chosen { node, why, .. } => p.chosen = Some(format!("{}: {why}", node.label)),
            ModelsEvent::Progress { peer, label, done, total } => {
                p.progress.insert(peer, (label, done, total));
            }
            ModelsEvent::NodeDone { node, result } => match result {
                Ok(done) => {
                    if let Some(e) = p.progress.get_mut(&node.peer) {
                        e.1 = e.2;
                    }
                    p.results.push(format!(
                        "{}: {} ({}) {}",
                        node.label,
                        done.id,
                        human_bytes(done.size),
                        if done.served { "downloaded and serving" } else { "downloaded" }
                    ));
                }
                Err(m) => p.results.push(m),
            },
            ModelsEvent::Done => {
                p.finished = true;
                p.ended = Some("Done".into());
                self.models_reload(cx);
            }
            ModelsEvent::Cancelled => {
                p.finished = true;
                p.ended = Some("Cancelled. What was downloaded is kept; pull again to resume.".into());
                self.models_reload(cx);
            }
            ModelsEvent::Failed { message } => {
                p.finished = true;
                p.ended = Some(message);
            }
            _ => {}
        }
    }

    pub fn models_cancel_pull(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.models.pull.as_ref().filter(|p| !p.finished) {
            self.core.cancel(&p.run);
        }
        cx.notify();
    }

    pub fn models_dismiss_pull(&mut self, cx: &mut Context<Self>) {
        if self.models.pull.as_ref().is_some_and(|p| p.finished) {
            self.models.pull = None;
        }
        cx.notify();
    }

    pub fn models_ask_remove(&mut self, peer: &str, id: &str, cx: &mut Context<Self>) {
        let found = self.models.nodes.iter().flatten().find(|n| n.node.peer == peer).and_then(|n| {
            n.models.iter().find(|m| m.id == id).map(|m| RemoveAsk { peer: peer.to_string(), node: n.node.label.clone(), model: m.clone() })
        });
        self.models.remove = found;
        cx.notify();
    }

    pub fn models_cancel_remove(&mut self, cx: &mut Context<Self>) {
        self.models.remove = None;
        cx.notify();
    }

    pub fn models_confirm_remove(&mut self, cx: &mut Context<Self>) {
        let Some(ask) = self.models.remove.take() else { return };
        let core = self.core.clone();
        let (peer, id) = (ask.peer.clone(), ask.model.id.clone());
        let job = self.core.handle().spawn(async move { core.models_remove(&peer, &id, true).await });
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let r = job.await;
            let _ = this.update(cx, |v, cx| {
                v.models.notice = Some(match r {
                    Ok(Ok(freed)) => format!("Removed {} from {}: {} freed.", ask.model.id, ask.node, human_bytes(freed)),
                    Ok(Err(e)) => e,
                    Err(e) => e.to_string(),
                });
                v.models_reload(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    // ---- drawing --------------------------------------------------------------------------

    fn badge(&self, label: &str, text: String, color: u32) -> gpui::Div {
        let t = self.theme;
        div()
            .px(px(8.))
            .py(px(2.))
            .rounded(px(10.))
            .border_1()
            .border_color(rgb(color))
            .bg(rgb(t.background))
            .text_size(px(11.))
            .text_color(rgb(color))
            .child(format!("{label}: {text}"))
    }

    fn fit_badge(&self, node: &NodeModels, fit: Option<&NodeFit>) -> gpui::Div {
        let t = self.theme;
        let label = node.node.label.as_str();
        if !ready(node) {
            return self.badge(label, status_text(&node.status).to_lowercase(), t.warning);
        }
        match fit {
            None => self.badge(label, "checking…".into(), t.text_secondary),
            Some(f) => match f.verdict() {
                Verdict::Fits => self.badge(label, format!("fits {} ({})", f.best.clone().unwrap_or_default(), human_bytes(f.best_size)), t.success),
                Verdict::TooBig => self.badge(label, format!("too big for {}", human_bytes(f.usable_bytes)), t.error),
                Verdict::NoBudget => self.badge(label, "no memory budget; name a quant".into(), t.warning),
                _ => self.badge(label, "unknown".into(), t.text_secondary),
            },
        }
    }

    pub(super) fn models_screen(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = self.theme;
        let m = &self.models;
        let heading = |s: &str| div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child(s.to_string());
        let mut col = div().id("models-screen").track_scroll(&self.models_scroll).flex().flex_col().gap(px(10.)).px(px(24.)).py(px(16.)).flex_1().overflow_y_scroll();
        col = col.child(div().text_size(px(18.)).child("Models"));
        if let Some(e) = &m.error {
            col = col.child(div().text_size(px(12.)).text_color(rgb(t.error)).child(e.clone()));
        }
        if m.owner_blocked() {
            col = col.child(
                div()
                    .id("models-owner-banner")
                    .p(px(12.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(t.warning))
                    .bg(rgb(t.surface))
                    .text_size(px(13.))
                    .child("Your nodes only let devices linked to their owner manage models, and none accepted this device. Link this device from a node you own, then come back. Nothing here can change until then."),
            );
        }
        col = col.child(heading("YOUR NODES"));
        match &m.nodes {
            None => col = col.child(div().text_size(px(12.)).text_color(rgb(t.text_secondary)).child("Asking your devices…")),
            Some(nodes) if nodes.is_empty() => {
                col = col.child(div().text_size(px(12.)).text_color(rgb(t.text_secondary)).child("No other devices on your roster yet. Link a Ymir node first."))
            }
            Some(nodes) => {
                for n in nodes {
                    let color = match n.status {
                        NodeStatus::Ready => t.success,
                        NodeStatus::NotModelNode => t.text_secondary,
                        _ => t.warning,
                    };
                    let mut card = div()
                        .id(SharedString::from(format!("node-{}", n.node.peer)))
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(px(8.))
                        .bg(rgb(t.surface))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .child(div().flex_1().text_size(px(14.)).child(n.node.label.clone()))
                                .child(div().text_size(px(12.)).text_color(rgb(color)).child(status_text(&n.status))),
                        );
                    if let NodeStatus::NeedsOwnerDevice(r) | NodeStatus::Unreachable(r) = &n.status {
                        card = card.child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child(r.clone()));
                    }
                    if ready(n) && n.models.is_empty() {
                        card = card.child(div().text_size(px(12.)).text_color(rgb(t.text_secondary)).child("No models downloaded."));
                    }
                    for sm in &n.models {
                        let (peer, id) = (n.node.peer.clone(), sm.id.clone());
                        let served = match (&sm.served_as, &sm.state) {
                            (Some(_), Some(s)) => format!(" · serving ({s})"),
                            (Some(_), None) => " · serving".to_string(),
                            _ => String::new(),
                        };
                        card = card.child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.))
                                .child(div().flex_1().text_size(px(13.)).child(format!(
                                    "{} · {}{}{}",
                                    sm.id,
                                    human_bytes(sm.size),
                                    if sm.complete { "" } else { " · incomplete" },
                                    served
                                )))
                                .child(
                                    div()
                                        .id(SharedString::from(format!("remove-{}-{}", n.node.peer, sm.id)))
                                        .px(px(8.))
                                        .py(px(3.))
                                        .rounded(px(6.))
                                        .text_size(px(12.))
                                        .bg(rgb(t.surface_variant))
                                        .text_color(rgb(t.error))
                                        .cursor_pointer()
                                        .child("Remove…")
                                        .on_click(cx.listener(move |v, _, _, cx| v.models_ask_remove(&peer, &id, cx))),
                                ),
                        );
                    }
                    col = col.child(card);
                }
            }
        }

        col = col.child(heading("FIND A MODEL"));
        let blocked = m.owner_blocked();
        col = col.child(
            div()
                .flex()
                .flex_row()
                .gap(px(8.))
                .child(
                    div().flex_1().px(px(10.)).py(px(8.)).rounded(px(6.)).bg(rgb(t.surface)).text_size(px(13.)).child(self.models_query.clone()),
                )
                .child(
                    self.button("models-search", if m.searching { "Searching…" } else { "Search" }, true)
                        .when(blocked || m.searching, |d| d.opacity(0.5))
                        .on_click(cx.listener(|v, _, _, cx| v.models_search(cx))),
                ),
        );
        // Where a pull goes.
        let mut picker = div().flex().flex_row().flex_wrap().items_center().gap(px(6.)).child(div().text_size(px(12.)).child("Pull to"));
        let mut options: Vec<(SharedString, String, Pick)> = vec![
            ("pick-auto".into(), "Auto (best fit)".into(), Pick::Auto),
            ("pick-all".into(), "All nodes".into(), Pick::All),
        ];
        for n in m.ready_nodes() {
            options.push((format!("pick-{}", n.node.peer).into(), n.node.label.clone(), Pick::Node(n.node.peer.clone())));
        }
        for (id, label, pick) in options {
            let on = m.pick == pick;
            picker = picker.child(self.button(id, label, on).h(px(26.)).on_click(cx.listener(move |v, _, _, cx| v.models_pick(pick.clone(), cx))));
        }
        picker = picker.child(
            self.button("models-serve", if m.serve { "Serve now: on" } else { "Serve now: off" }, m.serve)
                .h(px(26.))
                .on_click(cx.listener(|v, _, _, cx| v.models_toggle_serve(cx))),
        );
        col = col.child(picker);
        if let Some(n) = &m.notice {
            col = col.child(div().id("models-notice").text_size(px(12.)).text_color(rgb(t.text_secondary)).child(n.clone()));
        }
        if let Some(p) = &m.pull {
            col = col.child(self.pull_card(p, cx));
        }
        for h in &m.hits {
            let repo = h.repo.clone();
            let fits = m.fits.get(&h.repo);
            let mut badges = div().flex().flex_row().flex_wrap().gap(px(6.));
            for n in m.nodes.iter().flatten() {
                let fit = fits.and_then(|f| f.iter().find(|f| f.node.peer == n.node.peer));
                badges = badges.child(div().id(SharedString::from(format!("fit-{}-{}", h.repo, n.node.peer))).child(self.fit_badge(n, fit)));
            }
            let (r1, r2) = (repo.clone(), repo.clone());
            let quant = m.quant.get(&h.repo).cloned().unwrap_or_else(|| "Best that fits".into());
            let busy = m.pull.as_ref().is_some_and(|p| !p.finished);
            col = col.child(
                div()
                    .id(SharedString::from(format!("hit-{}", h.repo)))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(rgb(t.surface))
                    .child(div().text_size(px(14.)).child(h.repo.clone()))
                    .child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child(format!(
                        "{} downloads · {} likes{}",
                        count(h.downloads),
                        count(h.likes),
                        if h.gated { " · gated (needs a Hub token on the node)" } else { "" }
                    )))
                    .child(badges)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(8.))
                            .items_center()
                            .child(
                                self.button(SharedString::from(format!("quant-{}", h.repo)), format!("Quant: {quant}"), false)
                                    .h(px(26.))
                                    .on_click(cx.listener(move |v, _, _, cx| v.models_cycle_quant(&r1, cx))),
                            )
                            .child(div().flex_1())
                            .child(
                                self.button(SharedString::from(format!("pull-{}", h.repo)), "Pull", true)
                                    .when(busy || blocked, |d| d.opacity(0.5))
                                    .on_click(cx.listener(move |v, _, _, cx| v.models_pull(&r2, cx))),
                            ),
                    ),
            );
        }
        col
    }

    fn pull_card(&self, p: &PullView, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = self.theme;
        let mut card = div().id("pull-card").flex().flex_col().gap(px(6.)).p(px(10.)).rounded(px(8.)).border_1().border_color(rgb(t.border)).bg(rgb(t.surface)).child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .child(div().flex_1().text_size(px(14.)).child(format!("Pulling {}", p.repo)))
                .child(if p.finished {
                    self.button("pull-dismiss", "Dismiss", false).h(px(26.)).on_click(cx.listener(|v, _, _, cx| v.models_dismiss_pull(cx)))
                } else {
                    self.button("pull-cancel", "Cancel", false).h(px(26.)).on_click(cx.listener(|v, _, _, cx| v.models_cancel_pull(cx)))
                }),
        );
        if let Some(c) = &p.chosen {
            card = card.child(div().id("pull-chosen").text_size(px(12.)).text_color(rgb(t.text_secondary)).child(format!("Auto picked {c}")));
        }
        for (peer, (label, done, total)) in &p.progress {
            let frac = if *total == 0 { 0.0 } else { (*done as f32 / *total as f32).clamp(0.0, 1.0) };
            card = card.child(
                div()
                    .id(SharedString::from(format!("pull-progress-{peer}")))
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(div().text_size(px(12.)).child(format!("{label}: {} of {} ({:.0}%)", human_bytes(*done), human_bytes(*total), frac * 100.0)))
                    .child(
                        div()
                            .h(px(6.))
                            .w_full()
                            .rounded(px(3.))
                            .bg(rgb(t.surface_variant))
                            .child(div().h_full().rounded(px(3.)).bg(rgb(t.accent)).w(gpui::relative(frac))),
                    ),
            );
        }
        for r in &p.results {
            card = card.child(div().text_size(px(12.)).child(r.clone()));
        }
        if let Some(e) = &p.ended {
            card = card.child(div().id("pull-ended").text_size(px(12.)).text_color(rgb(t.text_secondary)).child(e.clone()));
        }
        if p.finished && p.results.iter().any(|r| r.contains("serving")) {
            card = card.child(
                div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child("Serving lasts until the node restarts. To keep it, add it to that node's node.toml."),
            );
        }
        card
    }

    /// The remove confirm card over the window.
    pub(super) fn models_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let t = self.theme;
        let ask = self.models.remove.as_ref()?;
        let danger = self.button("models-remove-confirm", "Remove", true).bg(rgb(t.error)).text_color(rgb(t.background));
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(0x00000099))
                .child(
                    div()
                        .id("remove-dialog")
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .w(px(420.))
                        .p(px(20.))
                        .rounded(px(12.))
                        .border_1()
                        .border_color(rgb(t.border))
                        .bg(rgb(t.surface))
                        .child(div().text_size(px(16.)).child(format!("Remove {}?", ask.model.id)))
                        .child(div().text_size(px(13.)).text_color(rgb(t.text_secondary)).child(format!(
                            "Deletes {} from {}{}. You can pull it again later.",
                            human_bytes(ask.model.size),
                            ask.node,
                            if ask.model.served_as.is_some() { " and stops serving it" } else { "" }
                        )))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(8.))
                                .justify_end()
                                .child(self.button("models-remove-cancel", "Cancel", false).on_click(cx.listener(|v, _, _, cx| v.models_cancel_remove(cx))))
                                .child(danger.on_click(cx.listener(|v, _, _, cx| v.models_confirm_remove(cx)))),
                        ),
                )
                .into_any_element(),
        )
    }
}

fn count(n: u64) -> String {
    match n {
        1_000_000.. => format!("{:.1}M", n as f64 / 1e6),
        1_000.. => format!("{:.1}k", n as f64 / 1e3),
        _ => n.to_string(),
    }
}
