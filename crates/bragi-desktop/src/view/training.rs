//! The Training view, a toggle on the Models screen: what each of your Ymir model nodes has
//! trained and is training, and the inference nodes' rollouts with each version's share, the
//! canary and the run that trained it. A read through `Core::training_overview`; nothing here
//! changes a node (the Flutter app gets the same view once its bridge is regenerated).

use bragi_core::NodeStatus;
use bragi_core::models::training::{Stage, TrainingOverview};

use super::*;

#[derive(Default)]
pub(super) struct Training {
    /// The toggle: the Models screen shows training instead of models.
    pub open: bool,
    pub overview: Option<TrainingOverview>,
    pub error: Option<String>,
}

/// `YYYY-MM-DD HH:MM` UTC from Unix milliseconds (no date crate in the shell).
fn utc_minute(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02} UTC", rem / 3600, rem % 3600 / 60)
}

pub(super) fn stage_text(s: Stage) -> &'static str {
    match s {
        Stage::Live => "live",
        Stage::Canary => "canary",
        Stage::Standby => "standby",
    }
}

impl BragiView {
    /// The Models screen's toggle between models and training.
    pub fn toggle_training(&mut self, on: bool, cx: &mut Context<Self>) {
        self.training.open = on;
        if on {
            self.training_reload(cx);
        }
        cx.notify();
    }

    /// Asks every device for its training and rollouts.
    pub fn training_reload(&mut self, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let job = self.core.handle().spawn(async move { core.training_overview().await });
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let r = job.await;
            let _ = this.update(cx, |v, cx| {
                match r {
                    Ok(Ok(o)) => {
                        v.training.overview = Some(o);
                        v.training.error = None;
                    }
                    Ok(Err(e)) => v.training.error = Some(e.message),
                    Err(e) => v.training.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn models_tabs(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_row()
            .gap(px(8.))
            .child(self.button("models-tab-models", "Models", !self.training.open).on_click(cx.listener(|v, _, _, cx| v.toggle_training(false, cx))))
            .child(
                self.button("models-tab-training", "Training", self.training.open).on_click(cx.listener(|v, _, _, cx| v.toggle_training(true, cx))),
            )
            .when(self.training.open, |d| {
                d.child(self.button("training-refresh", "Refresh", false).on_click(cx.listener(|v, _, _, cx| v.training_reload(cx))))
            })
    }

    pub(super) fn training_section(&self) -> gpui::Div {
        let t = self.theme;
        let heading = |s: &str| div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child(s.to_string());
        let small = |s: String| div().text_size(px(12.)).text_color(rgb(t.text_secondary)).child(s);
        let card = || div().flex().flex_col().gap(px(2.)).px(px(10.)).py(px(6.)).rounded(px(8.)).bg(rgb(t.surface));
        let mut col = div().flex().flex_col().gap(px(10.));
        if let Some(e) = &self.training.error {
            col = col.child(div().text_size(px(12.)).text_color(rgb(t.error)).child(e.clone()));
        }
        let Some(o) = &self.training.overview else {
            return col.child(small("Asking your devices…".into()));
        };
        col = col.child(heading("ROLLOUTS"));
        if o.rollouts.is_empty() {
            col = col.child(small("No versioned rollouts on your inference nodes.".into()));
        }
        for r in &o.rollouts {
            let mut c = card().child(div().text_size(px(14.)).child(format!("{} on {}", r.name, r.node.label)));
            for v in &r.versions {
                let color = match v.stage {
                    Stage::Live => t.success,
                    Stage::Canary => t.warning,
                    Stage::Standby => t.text_secondary,
                };
                let trained = v.trained_by.as_ref().map(|(n, run)| format!(" · trained on {n}, run {run}")).unwrap_or_default();
                c = c.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(8.))
                        .child(div().flex_1().text_size(px(13.)).child(format!("v{} · {} · {:.0}%{trained}", v.version, v.model, v.percent)))
                        .child(div().text_size(px(12.)).text_color(rgb(color)).child(stage_text(v.stage))),
                );
            }
            col = col.child(c);
        }
        col = col.child(heading("RUNS AND JOBS"));
        let ready: Vec<_> = o.ready().collect();
        if ready.is_empty() {
            col = col.child(small("None of your devices reports training (it needs a Ymir model node, 0.15.5 or later).".into()));
        }
        for n in ready {
            let mut c = card().child(
                div().flex().flex_row().child(div().flex_1().text_size(px(14.)).child(n.node.label.clone())).child(small(if n.remote {
                    "takes runs from your devices".into()
                } else {
                    "trains locally only".into()
                })),
            );
            if n.runs.is_empty() && n.jobs.is_empty() {
                c = c.child(small("No training runs.".into()));
            }
            for j in &n.jobs {
                let at = j.checkpoint_epoch.map(|e| format!(", checkpoint after epoch {e}")).unwrap_or_default();
                let on = j.holder.as_ref().map(|h| format!(" on {h}")).unwrap_or_default();
                c = c.child(
                    div()
                        .text_size(px(13.))
                        .child(format!("job {} · {}{on} · {} → {} (attempt {}{at})", j.id, j.state, j.base, j.adapter, j.attempts)),
                );
            }
            for r in &n.runs {
                let adapter = r.adapter.as_ref().map(|a| format!(" → {a}")).unwrap_or_default();
                let loss = r.final_loss.map(|l| format!(" · loss {l:.3}")).unwrap_or_default();
                c = c.child(div().text_size(px(13.)).child(format!("{} · {} · {}{adapter}{loss}", utc_minute(r.started_ms), r.status, r.base)));
                c = c.child(small(format!("{} rows of {} ({}), {} epochs", r.rows_used, r.dataset, r.dataset_license, r.epochs)));
                if let Some(e) = &r.error {
                    c = c.child(div().text_size(px(11.)).text_color(rgb(t.error)).child(e.clone()));
                }
            }
            col = col.child(c);
        }
        let others = o.nodes.iter().filter(|n| !n.status.is_ready()).count();
        if others > 0 {
            let refused = o.nodes.iter().filter(|n| matches!(n.status, NodeStatus::NeedsOwnerDevice(_))).count();
            let words = if refused > 0 { format!(" ({refused} refused this device)") } else { String::new() };
            col = col.child(small(format!("{others} other devices report no training{words}.")));
        }
        col
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates_are_utc() {
        assert_eq!(super::utc_minute(0), "1970-01-01 00:00 UTC");
        assert_eq!(super::utc_minute(1_764_072_000_000), "2025-11-25 12:00 UTC");
    }
}
