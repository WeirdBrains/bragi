//! Settings, On-device model (B4): off by default; each model downloads only after its consent
//! card (what, how big, from where) gets an OK; a downloaded, chosen model shows in the target
//! picker as "On this device". The core does the work (`bragi_core::on_device`); this only asks,
//! shows and refreshes, the same flow as the TUI and the Flutter app.

use bragi_core::OnDeviceModel;

use super::*;

/// What a model's row offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModelAction {
    Download,
    Stop,
    Use,
    Remove,
}

impl ModelAction {
    fn label(self) -> &'static str {
        match self {
            ModelAction::Download => "Download",
            ModelAction::Stop => "Stop",
            ModelAction::Use => "Use",
            ModelAction::Remove => "Remove",
        }
    }
}

/// The buttons on `m`'s row while `downloading` (a model id) downloads.
pub(super) fn model_actions(m: &OnDeviceModel, downloading: Option<&str>) -> Vec<ModelAction> {
    if downloading == Some(m.id.as_str()) {
        return vec![ModelAction::Stop];
    }
    let mut v = Vec::new();
    if !m.installed && downloading.is_none() {
        v.push(ModelAction::Download);
    }
    if m.installed && !m.chosen {
        v.push(ModelAction::Use);
    }
    if m.installed || m.partial > 0 {
        v.push(ModelAction::Remove);
    }
    v
}

/// The note under a model's name.
pub(super) fn model_note(m: &OnDeviceModel) -> String {
    let state = match (m.installed, m.partial > 0) {
        (true, _) => "downloaded",
        (false, true) => "partly downloaded",
        (false, false) => "not downloaded",
    };
    format!("{} · {state}{}", m.size_label, if m.chosen { " · in use" } else { "" })
}

impl BragiView {
    fn after_on_device(&mut self, r: bragi_core::Result<()>, cx: &mut Context<Self>) {
        if let Err(e) = r {
            self.notice = Some(e.message);
        }
        self.state.on_device = self.core.on_device_status();
        self.refresh_prefs();
        cx.notify();
    }

    pub fn toggle_on_device(&mut self, cx: &mut Context<Self>) {
        let on = !self.core.on_device_settings().enabled;
        let r = self.core.set_on_device(on).map(|_| ());
        self.after_on_device(r, cx);
    }

    /// Opens the consent card for `id`; nothing downloads until its OK.
    pub fn ask_on_device_download(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.core.on_device_consent(id) {
            Ok(lines) => self.prefs.on_device_consent = Some((id.to_string(), lines)),
            Err(e) => self.notice = Some(e.message),
        }
        cx.notify();
    }

    pub fn confirm_on_device_download(&mut self, cx: &mut Context<Self>) {
        let Some((id, _)) = self.prefs.on_device_consent.take() else { return };
        let r = self.core.download_on_device_model(&id, true);
        self.after_on_device(r, cx);
    }

    fn on_device_action(&mut self, id: &str, a: ModelAction, cx: &mut Context<Self>) {
        let r = match a {
            ModelAction::Download => return self.ask_on_device_download(id, cx),
            ModelAction::Stop => {
                self.core.cancel_on_device_download();
                Ok(())
            }
            ModelAction::Use => self.core.choose_on_device_model(id),
            ModelAction::Remove => self.core.remove_on_device_model(id).map(|_| ()),
        };
        self.after_on_device(r, cx);
    }

    pub(super) fn on_device_section(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = self.theme;
        let st = &self.state.on_device;
        let row = |title: String, note: String| {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.))
                .p(px(12.))
                .rounded(px(8.))
                .bg(rgb(t.surface))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(14.)).child(title))
                        .child(div().text_size(px(12.)).text_color(rgb(t.text_secondary)).child(note)),
                )
        };
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child("ON-DEVICE MODEL"))
            .child(
                row("Run a model on this device".into(), st.line()).child(
                    self.button("on-device", if st.enabled { "On" } else { "Off" }, st.enabled)
                        .w(px(64.))
                        .on_click(cx.listener(|v, _, _, cx| v.toggle_on_device(cx))),
                ),
            );
        if !st.enabled {
            return col;
        }
        let downloading = st.download.as_ref().map(|d| d.id.clone());
        for (i, m) in self.core.on_device_models().into_iter().enumerate() {
            let mut r = row(m.label.clone(), model_note(&m));
            for a in model_actions(&m, downloading.as_deref()) {
                let id = m.id.clone();
                r = r.child(
                    self.button(SharedString::from(format!("on-device-{}-{i}", a.label().to_lowercase())), a.label(), a == ModelAction::Use)
                        .on_click(cx.listener(move |v, _, _, cx| v.on_device_action(&id, a, cx))),
                );
            }
            col = col.child(r);
        }
        col
    }

    pub(super) fn on_device_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let (_, lines) = self.prefs.on_device_consent.as_ref()?;
        let t = self.theme;
        let mut text = div().id("on-device-consent").flex().flex_col().gap(px(4.));
        for l in lines {
            text = text.child(div().text_size(px(13.)).text_color(rgb(t.text_secondary)).child(l.clone()));
        }
        let c = self.card("Download this model?").child(text).child(
            div()
                .flex()
                .flex_row()
                .gap(px(8.))
                .justify_end()
                .child(self.button("on-device-cancel", "Cancel", false).on_click(cx.listener(|v, _, _, cx| {
                    v.prefs.on_device_consent = None;
                    cx.notify();
                })))
                .child(self.button("on-device-ok", "Download", true).on_click(cx.listener(|v, _, _, cx| v.confirm_on_device_download(cx)))),
        );
        Some(self.modal("on-device-confirm", c).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(installed: bool, partial: u64, chosen: bool) -> OnDeviceModel {
        OnDeviceModel {
            id: "gemma-4-e2b".into(),
            label: "Gemma 4 E2B instruct (Q4_0)".into(),
            size: 2_841_481_184,
            size_label: "2.84 GB".into(),
            installed,
            partial,
            chosen,
        }
    }

    #[test]
    fn rows_offer_what_fits_their_state() {
        use ModelAction::*;
        assert_eq!(model_actions(&m(false, 0, true), None), [Download]);
        assert_eq!(model_actions(&m(false, 10, false), None), [Download, Remove]);
        assert_eq!(model_actions(&m(false, 10, false), Some("gemma-4-e2b")), [Stop]);
        // One download at a time.
        assert_eq!(model_actions(&m(false, 0, false), Some("gemma-4-e4b")), Vec::<ModelAction>::new());
        assert_eq!(model_actions(&m(true, 0, false), None), [Use, Remove]);
        assert_eq!(model_actions(&m(true, 0, true), None), [Remove]);
        assert_eq!(model_note(&m(true, 0, true)), "2.84 GB · downloaded · in use");
        assert_eq!(model_note(&m(false, 5, false)), "2.84 GB · partly downloaded");
    }
}
