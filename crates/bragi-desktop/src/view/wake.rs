//! Settings, Wake-ups (B4): push on by default with an off switch, and the opt-in relay wake
//! path, which asks first with its battery cost. The core does the work (`bragi_core::wake`);
//! this only asks, shows and refreshes, the same flow as the Flutter app.

use bragi_core::wake::{RELAY_WAKE_COST, WakeSettings};

use super::*;

impl BragiView {
    pub fn set_wake(&mut self, s: WakeSettings, cx: &mut Context<Self>) {
        self.prefs.wake_confirm = false;
        if let Err(e) = self.core.set_wake_settings(s) {
            self.notice = Some(e.message);
        }
        self.state.wake = self.core.wake_status();
        cx.notify();
    }

    pub fn toggle_wake_push(&mut self, cx: &mut Context<Self>) {
        let now = self.core.wake_settings();
        self.set_wake(WakeSettings { push: !now.push, ..now }, cx);
    }

    /// Relay wake off goes straight off; on asks first (see [`RELAY_WAKE_COST`]).
    pub fn toggle_relay_wake(&mut self, cx: &mut Context<Self>) {
        let now = self.core.wake_settings();
        if !now.push {
            return;
        }
        if now.relay_wake {
            self.set_wake(WakeSettings { relay_wake: false, ..now }, cx);
        } else {
            self.prefs.wake_confirm = true;
            cx.notify();
        }
    }

    pub fn confirm_relay_wake(&mut self, cx: &mut Context<Self>) {
        let now = self.core.wake_settings();
        self.set_wake(WakeSettings { relay_wake: true, ..now }, cx);
    }

    pub(super) fn wake_section(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = self.theme;
        let w = &self.state.wake.settings;
        let relay_note = match (w.push, w.relay_wake, self.state.wake.relay_connected) {
            (false, _, _) => "Needs wake-ups on.",
            (true, false, _) => "Off. Works without Google or Apple push, but keeps a connection open and uses more battery.",
            (true, true, true) => "On: connected to your relay. Uses more battery.",
            (true, true, false) => "On: connecting to your relay…",
        };
        let row = |title: &str, note: &str, button: gpui::Stateful<gpui::Div>| {
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
                        .child(div().text_size(px(14.)).child(title.to_string()))
                        .child(div().text_size(px(12.)).text_color(rgb(t.text_secondary)).child(note.to_string())),
                )
                .child(button)
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child("WAKE-UPS"))
            .child(row(
                "Wake-ups",
                "Your other linked devices can wake this one to sync. A wake carries no message content.",
                self.switch("wake-push", w.push).on_click(cx.listener(|v, _, _, cx| v.toggle_wake_push(cx))),
            ))
            .child(row(
                "Wake through your relay",
                relay_note,
                self.switch("wake-relay", w.push && w.relay_wake).on_click(cx.listener(|v, _, _, cx| v.toggle_relay_wake(cx))),
            ))
    }

    fn switch(&self, id: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
        self.button(id, if on { "On" } else { "Off" }, on).w(px(64.))
    }

    pub(super) fn wake_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        if !self.prefs.wake_confirm {
            return None;
        }
        let t = self.theme;
        let c = self
            .card("Wake through your relay?")
            .child(div().id("relay-wake-cost").text_size(px(13.)).text_color(rgb(t.text_secondary)).child(RELAY_WAKE_COST))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.))
                    .justify_end()
                    .child(self.button("relay-wake-cancel", "Cancel", false).on_click(cx.listener(|v, _, _, cx| {
                        v.prefs.wake_confirm = false;
                        cx.notify();
                    })))
                    .child(self.button("relay-wake-ok", "Turn on", true).on_click(cx.listener(|v, _, _, cx| v.confirm_relay_wake(cx)))),
            );
        Some(self.modal("relay-wake-confirm", c).into_any_element())
    }
}
