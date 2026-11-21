//! Deleting history in the window: a message's hover Delete (or right-click), the × on a
//! conversation in the list, Clear all history in Settings, the confirm card, and the Undo bar.
//! Each delete is one `Core` call (bragi-core's `history::forget`); this only asks first, the same
//! questions the Flutter app asks.

use super::*;

/// The word Clear all history wants typed (the Flutter app's too).
pub(super) const CLEAR_WORD: &str = "delete";

/// A delete waiting for the user to confirm it.
pub(super) enum Confirm {
    Message { conversation: String, id: String, has_reply: bool },
    Conversation { id: String, title: String },
    ClearAll,
}

impl BragiView {
    fn streaming_here(&self) -> bool {
        self.streaming.as_ref().is_some_and(|s| self.current.as_ref().is_some_and(|c| c.id == s.conversation))
    }

    pub fn ask_delete_message(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.streaming_here() {
            self.notice = Some("Stop the answer before deleting messages.".into());
        } else if let Some(c) = &self.current {
            let has_reply = c.reply_to(id).is_some();
            self.confirm = Some(Confirm::Message { conversation: c.id.clone(), id: id.to_string(), has_reply });
        }
        cx.notify();
    }

    pub fn ask_delete_conversation(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.streaming.as_ref().is_some_and(|s| s.conversation == id) {
            self.notice = Some("Stop the answer before deleting this conversation.".into());
        } else if let Some(s) = self.conversations.iter().find(|c| c.id == id) {
            self.confirm = Some(Confirm::Conversation { id: id.to_string(), title: s.title.clone() });
        }
        cx.notify();
    }

    pub fn ask_clear(&mut self, cx: &mut Context<Self>) {
        self.confirm_input.update(cx, |i, cx| i.set_text("", cx));
        self.confirm = Some(Confirm::ClearAll);
        cx.notify();
    }

    pub fn cancel_confirm(&mut self, cx: &mut Context<Self>) {
        self.confirm = None;
        cx.notify();
    }

    pub fn toggle_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_open = !self.settings_open;
        cx.notify();
    }

    /// The confirm card's yes; `with_reply` also deletes a question's answer.
    pub fn confirm_delete(&mut self, with_reply: bool, cx: &mut Context<Self>) {
        let done = match self.confirm.take() {
            None => return,
            Some(Confirm::Message { conversation, id, .. }) => self.core.delete_message(&conversation, &id, with_reply),
            Some(Confirm::Conversation { id, .. }) => {
                let d = self.core.delete_conversations(std::slice::from_ref(&id));
                if d.is_ok() && self.current.as_ref().is_some_and(|c| c.id == id) {
                    self.current = None;
                }
                d
            }
            Some(Confirm::ClearAll) => {
                if !self.confirm_input.read(cx).text().trim().eq_ignore_ascii_case(CLEAR_WORD) {
                    self.confirm = Some(Confirm::ClearAll);
                    cx.notify();
                    return;
                }
                if let Some(s) = &self.streaming {
                    self.core.cancel(&s.request_id);
                }
                self.deleted = None;
                self.notice = Some(match self.core.clear_history() {
                    Ok(1) => "Deleted 1 conversation.".into(),
                    Ok(n) => format!("Deleted {n} conversations."),
                    Err(e) => e.message,
                });
                self.current = None;
                self.settings_open = false;
                self.reload();
                cx.notify();
                return;
            }
        };
        match done {
            Ok(d) => {
                self.notice = None;
                self.deleted = Some(d);
                self.expire_undo(cx);
            }
            Err(e) => self.notice = Some(e.message),
        }
        self.reload();
        cx.notify();
    }

    pub fn undo_delete(&mut self, cx: &mut Context<Self>) {
        if let Some(d) = self.deleted.take()
            && let Err(e) = self.core.undo_delete(&d.undo)
        {
            self.notice = Some(e.message);
        }
        self.reload();
        cx.notify();
    }

    /// Hides the Undo bar a little before the core's undo window closes.
    fn expire_undo(&mut self, cx: &mut Context<Self>) {
        let Some(token) = self.deleted.as_ref().map(|d| d.undo.clone()) else { return };
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(Duration::from_millis(bragi_core::UNDO_MS.saturating_sub(1500))).await;
            let _ = this.update(cx, |v, cx| {
                if v.deleted.as_ref().is_some_and(|d| d.undo == token) {
                    v.deleted = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    // ---- drawing ---------------------------------------------------------------------------

    /// A message with its Delete, shown on hover (right-click asks too). None while it streams.
    pub(super) fn message_row<E: IntoElement>(&self, i: usize, m: &ChatMessage, bubble: E, cx: &mut Context<Self>) -> impl IntoElement + use<E> {
        let t = self.theme;
        let group = SharedString::from(format!("msg-{i}"));
        let can = !m.id.is_empty() && !self.streaming_here();
        let (id1, id2) = (m.id.clone(), m.id.clone());
        let reactions = self.reactions(m, group.clone(), cx);
        div().id(SharedString::from(format!("row-{i}"))).group(group.clone()).flex().flex_col().gap(px(2.)).child(bubble).child(reactions).when(
            can,
            |d| {
                d.on_mouse_down(MouseButton::Right, cx.listener(move |v, _, _, cx| v.ask_delete_message(&id1, cx))).child(
                    div().flex().flex_row().when(m.role == Role::User, |d| d.justify_end()).child(
                        div()
                            .id(SharedString::from(format!("delete-msg-{i}")))
                            .px(px(6.))
                            .text_size(px(11.))
                            .text_color(rgb(t.text_secondary))
                            .opacity(0.)
                            .group_hover(group, |s| s.opacity(1.))
                            .hover(|s| s.text_color(rgb(t.error)))
                            .cursor_pointer()
                            .on_click(cx.listener(move |v, _, _, cx| v.ask_delete_message(&id2, cx)))
                            .child("Delete"),
                    ),
                )
            },
        )
    }

    /// The × on a conversation in the list.
    pub(super) fn conversation_delete(&self, id: &str, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = self.theme;
        let id = id.to_string();
        div()
            .id(SharedString::from(format!("delete-conv-{id}")))
            .flex_none()
            .px(px(4.))
            .text_size(px(13.))
            .text_color(rgb(t.text_secondary))
            .hover(|s| s.text_color(rgb(t.error)))
            .cursor_pointer()
            .on_click(cx.listener(move |v, _, _, cx| {
                cx.stop_propagation();
                v.ask_delete_conversation(&id, cx)
            }))
            .child("×")
    }

    /// "Deleted … · Undo", while the last delete can be undone.
    pub(super) fn undo_bar(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let t = self.theme;
        let d = self.deleted.as_ref()?;
        let what = match (d.conversations, d.messages) {
            (1, _) => "Deleted the conversation.".to_string(),
            (n, _) if n > 1 => format!("Deleted {n} conversations."),
            (_, 1) => "Deleted the message.".to_string(),
            (_, 2) => "Deleted the message and its answer.".to_string(),
            (_, n) => format!("Deleted {n} messages."),
        };
        Some(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.))
                .mx(px(16.))
                .my(px(6.))
                .px(px(12.))
                .py(px(6.))
                .rounded(px(8.))
                .bg(rgb(t.surface_variant))
                .child(div().flex_1().text_size(px(13.)).child(what))
                .child(self.button("undo", "Undo", true).on_click(cx.listener(|v, _, _, cx| v.undo_delete(cx)))),
        )
    }

    /// Settings: for now only history (the endpoints and sessions settings join it later).
    pub(super) fn settings(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = self.theme;
        div()
            .id("settings")
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(24.))
            .flex_1()
            .child(div().text_size(px(18.)).child("Settings"))
            .child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child("HISTORY"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.))
                    .p(px(12.))
                    .rounded(px(8.))
                    .bg(rgb(t.surface))
                    .child(
                        div().flex_1().flex().flex_col().child(div().text_size(px(14.)).child("Clear all history")).child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(t.text_secondary))
                                .child("Every conversation, on all your linked devices. This can't be undone."),
                        ),
                    )
                    .child(
                        self.button("clear-history", "Clear all history…", false)
                            .text_color(rgb(t.error))
                            .on_click(cx.listener(|v, _, _, cx| v.ask_clear(cx))),
                    ),
            )
            .child(self.button("settings-done", "Done", false).w(px(80.)).on_click(cx.listener(|v, _, _, cx| v.toggle_settings(cx))))
    }

    /// The confirm card over the window.
    pub(super) fn confirm_card(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let t = self.theme;
        let confirm = self.confirm.as_ref()?;
        let danger = |b: gpui::Stateful<gpui::Div>| b.bg(rgb(t.error)).text_color(rgb(t.background));
        let cancel = self.button("delete-cancel", "Cancel", false).on_click(cx.listener(|v, _, _, cx| v.cancel_confirm(cx)));
        let mut buttons = div().flex().flex_row().gap(px(8.)).justify_end().child(cancel);
        let (title, text) = match confirm {
            Confirm::Message { has_reply: true, .. } => {
                buttons = buttons
                    .child(self.button("delete-only", "Only this", false).on_click(cx.listener(|v, _, _, cx| v.confirm_delete(false, cx))))
                    .child(
                        danger(self.button("delete-with-reply", "With the answer", true))
                            .on_click(cx.listener(|v, _, _, cx| v.confirm_delete(true, cx))),
                    );
                (
                    "Delete this message?",
                    "Delete the answer that followed too? It goes from all your linked devices and is not sent to a model again.".to_string(),
                )
            }
            Confirm::Message { .. } => {
                buttons = buttons
                    .child(danger(self.button("delete-confirm", "Delete", true)).on_click(cx.listener(|v, _, _, cx| v.confirm_delete(false, cx))));
                ("Delete this message?", "It goes from all your linked devices and is not sent to a model again.".to_string())
            }
            Confirm::Conversation { title, .. } => {
                buttons = buttons
                    .child(danger(self.button("delete-confirm", "Delete", true)).on_click(cx.listener(|v, _, _, cx| v.confirm_delete(false, cx))));
                ("Delete this conversation?", format!("“{title}” goes from all your linked devices."))
            }
            Confirm::ClearAll => {
                let ready = self.confirm_input.read(cx).text().trim().eq_ignore_ascii_case(CLEAR_WORD);
                let b = self.button("clear-confirm", "Clear all history", true);
                let b = if ready { danger(b) } else { b.bg(rgb(t.surface_variant)).text_color(rgb(t.text_secondary)).opacity(0.6) };
                buttons = buttons.child(b.on_click(cx.listener(|v, _, _, cx| v.confirm_delete(false, cx))));
                ("Clear all history?", format!("Every conversation goes, on all your linked devices, with no undo. Type “{CLEAR_WORD}” to go ahead."))
            }
        };
        let mut card = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .w(px(420.))
            .p(px(20.))
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(t.border))
            .bg(rgb(t.surface))
            .child(div().text_size(px(16.)).child(title))
            .child(div().text_size(px(13.)).text_color(rgb(t.text_secondary)).child(text));
        if matches!(confirm, Confirm::ClearAll) {
            card =
                card.child(div().px(px(10.)).py(px(8.)).rounded(px(6.)).bg(rgb(t.background)).text_size(px(13.)).child(self.confirm_input.clone()));
        }
        Some(
            div()
                .id("confirm")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(gpui::rgba(0x000000aa))
                .occlude()
                .child(card.child(buttons)),
        )
    }
}
