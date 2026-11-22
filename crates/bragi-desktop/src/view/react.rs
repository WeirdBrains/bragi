//! Reactions in the window: the pills under a message, the quick bar on hover, and the picker.
//! Each click is one `Core::react` call (bragi-core's `reactions`); this only draws it.

use super::*;

impl BragiView {
    /// The reaction row under a message: its pills, and on hover (`group`) the quick bar and a "+"
    /// for the picker. A streaming answer takes none yet.
    pub(super) fn reactions(&self, m: &ChatMessage, group: SharedString, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme;
        let mine = m.role == Role::User;
        let can_react = !m.id.is_empty() && m.state != MessageState::Streaming;
        let mut under = div().flex().flex_row().gap(px(4.)).items_center().when(mine, |d| d.justify_end());
        for e in &m.reactions {
            let (id, emoji) = (m.id.clone(), e.clone());
            under = under.child(
                div()
                    .id(SharedString::from(format!("pill-{}-{e}", m.id)))
                    .px(px(7.))
                    .py(px(1.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgb(t.border))
                    .bg(rgb(t.selection))
                    .text_size(px(13.))
                    .cursor_pointer()
                    .child(e.clone())
                    .on_click(cx.listener(move |v, _, _, cx| v.react(&id, &emoji, cx))),
            );
        }
        if can_react {
            let mut bar = div().flex().flex_row().gap(px(2.)).opacity(0.).group_hover(group.clone(), |s| s.opacity(1.));
            for e in bragi_core::QUICK_REACTIONS {
                let (id, emoji) = (m.id.clone(), e.to_string());
                let on = m.reactions.iter().any(|r| r == e);
                bar = bar.child(
                    div()
                        .id(SharedString::from(format!("react-{}-{e}", m.id)))
                        .px(px(4.))
                        .rounded(px(6.))
                        .text_size(px(13.))
                        .cursor_pointer()
                        .when(on, |d| d.bg(rgb(t.selection)))
                        .hover(|s| s.bg(rgb(t.surface_variant)))
                        .child(e)
                        .on_click(cx.listener(move |v, _, _, cx| v.react(&id, &emoji, cx))),
                );
            }
            let id = m.id.clone();
            bar = bar.child(
                div()
                    .id(SharedString::from(format!("react-more-{}", m.id)))
                    .px(px(5.))
                    .rounded(px(6.))
                    .text_size(px(13.))
                    .text_color(rgb(t.text_secondary))
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(t.surface_variant)))
                    .child("+")
                    .on_click(cx.listener(move |v, _, _, cx| v.toggle_react_picker(&id, cx))),
            );
            under = under.child(bar);
        }
        if can_react && self.react_picker.as_deref() == Some(m.id.as_str()) {
            let mut more = div().flex().flex_row().flex_wrap().gap(px(2.)).max_w(px(320.)).when(mine, |d| d.self_end());
            for e in EXTRA_REACTIONS {
                let (id, emoji) = (m.id.clone(), e.to_string());
                more = more.child(
                    div()
                        .id(SharedString::from(format!("react-x-{}-{e}", m.id)))
                        .px(px(4.))
                        .rounded(px(6.))
                        .text_size(px(15.))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(t.surface_variant)))
                        .child(e)
                        .on_click(cx.listener(move |v, _, _, cx| v.react(&id, &emoji, cx))),
                );
            }
            return div().flex().flex_col().gap(px(3.)).child(under).child(more).into_any_element();
        }
        under.into_any_element()
    }
}
