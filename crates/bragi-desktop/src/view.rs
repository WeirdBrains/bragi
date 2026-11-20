//! The window: draws `bragi-core`'s [`AppState`] and the current conversation, and turns
//! clicks and keys into `Core` actions. No behaviour lives here.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use bragi_core::demo::{DemoNode, Faults};
use bragi_core::{
    AppState, Attachment, ChatEvent, ChatMessage, Choice, Config, Conversation, ConversationSummary, Core, LinkCheck, MessageState, ModelStatus,
    NodeSearch, PairState, Role, Transport,
};
use gpui::{
    App, Bounds, Context, Entity, Focusable, KeyBinding, MouseButton, ScrollHandle, SharedString, Task, WeakEntity, Window, WindowBounds,
    WindowDecorations, WindowOptions, actions, div, prelude::*, px, rgb, size,
};

use crate::markdown;
use crate::text_input::{self, TextInput, TextInputEvent};
use crate::theme::Theme;

actions!(bragi, [Quit, Stop, ChatPageUp, ChatPageDown, ChatTop, ChatBottom]);

/// The process exit code: 0 ok, 1 the scenario failed.
static EXIT: AtomicU8 = AtomicU8::new(0);

pub struct Launch {
    pub home: Option<PathBuf>,
    pub demo: bool,
    pub scenario: bool,
}

struct Streaming {
    request_id: String,
    conversation: String,
}

/// An agent task's approval card, waiting for the user.
struct Card {
    request_id: String,
    ask: u64,
    summary: String,
    tier: String,
}

/// A reply's text, whether it was parsed while streaming, and its parsed Markdown.
type ParsedReply = (String, bool, Rc<bragi_core::markdown::Document>);

pub struct BragiView {
    core: Core,
    theme: Theme,
    state: AppState,
    conversations: Vec<ConversationSummary>,
    current: Option<Conversation>,
    model: Option<String>,
    streaming: Option<Streaming>,
    prompt: Entity<TextInput>,
    link_input: Entity<TextInput>,
    link_check: Option<LinkCheck>,
    /// The link code was just copied (the Copy button says so for a moment).
    copied: bool,
    /// Files chosen for the next message, the path box that adds one, and agent mode.
    attachments: Vec<Attachment>,
    attach_input: Entity<TextInput>,
    attach_open: bool,
    agent: bool,
    card: Option<Card>,
    notice: Option<String>,
    /// Thinking sections the user opened or closed, by conversation and message index (the
    /// default: open while the model reasons, closed once the answer starts).
    thinking_flipped: std::collections::HashSet<(String, usize)>,
    /// When the streaming answer's first reasoning arrived, for the live elapsed time.
    thinking_since: Option<Instant>,
    scroll: ScrollHandle,
    /// Follow the newest text; false once the reader scrolls up.
    stick: bool,
    /// A scroll gesture moved the list: re-read whether it sits at the bottom on the next frame.
    recheck: bool,
    /// The scrollbar thumb is being dragged.
    dragging: bool,
    close_only: bool,
    /// The monospace family for code, and the code block or message just copied ("Copied").
    mono: SharedString,
    copied_block: Option<SharedString>,
    /// Each shown reply's parsed Markdown, by message key, kept until its text changes.
    markdown: RefCell<HashMap<String, ParsedReply>>,
    _tasks: Vec<Task<()>>,
}

/// Under a tiling window manager the title bar offers close only.
fn tiling_wm() -> bool {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    ["hyprland", "sway", "i3", "niri", "river", "bspwm", "qtile", "awesome", "dwm"].iter().any(|w| desktop.contains(w))
        || std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
        || std::env::var_os("SWAYSOCK").is_some()
        || std::env::var_os("I3SOCK").is_some()
}

/// Applies a delta at its offset (a continued or restarted answer lines up by offset).
fn splice(text: &mut String, offset: u64, delta: &str) {
    let keep: String = text.chars().take(offset as usize).collect();
    *text = keep + delta;
}

impl BragiView {
    fn new(core: Core, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let theme = Theme::default();
        let prompt = cx.new(|cx| TextInput::new("Ask anything…", theme, cx));
        let link_input = cx.new(|cx| TextInput::new("Link code or node address", theme, cx));
        let attach_input = cx.new(|cx| TextInput::new("Path of a text file…", theme, cx));
        cx.subscribe_in(&attach_input, window, |this, _, ev: &TextInputEvent, _, cx| {
            if let TextInputEvent::Submit { .. } = ev {
                this.attach_typed(cx);
            }
        })
        .detach();
        cx.subscribe_in(&prompt, window, |this, _, ev: &TextInputEvent, window, cx| match ev {
            TextInputEvent::Submit { .. } => this.send(window, cx),
            TextInputEvent::Cancel => this.stop(cx),
            _ => {}
        })
        .detach();
        let mut w = core.watch();
        let follow = cx.spawn(async move |this: WeakEntity<Self>, cx| {
            loop {
                let s = w.borrow_and_update().clone();
                if this.update(cx, |v, cx| v.on_state(s, cx)).is_err() {
                    return;
                }
                if w.changed().await.is_err() {
                    return;
                }
            }
        });
        let state = core.state();
        let conversations = core.conversations(0, 100).unwrap_or_default();
        window.focus(&prompt.focus_handle(cx), cx);
        Self {
            core,
            theme,
            state,
            conversations,
            current: None,
            model: None,
            streaming: None,
            prompt,
            link_input,
            link_check: None,
            copied: false,
            mono: markdown::pick_mono(&cx.text_system().all_font_names()),
            copied_block: None,
            markdown: RefCell::default(),
            attachments: Vec::new(),
            attach_input,
            attach_open: false,
            agent: false,
            card: None,
            notice: None,
            thinking_flipped: Default::default(),
            thinking_since: None,
            scroll: ScrollHandle::new(),
            stick: true,
            recheck: false,
            dragging: false,
            close_only: tiling_wm(),
            _tasks: vec![follow],
        }
    }

    fn on_state(&mut self, s: AppState, cx: &mut Context<Self>) {
        if let Some(m) = &self.model
            && !s.catalog.models.iter().any(|x| &x.id == m)
        {
            self.model = None;
        }
        self.state = s;
        // History from another device arrives with a state wake-up; redraw it unless an answer
        // is streaming in (that has its own redraws).
        if self.streaming.is_none() && self.state.online {
            self.reload();
        }
        cx.notify();
    }

    fn model(&self) -> Option<String> {
        self.model.clone().or_else(|| self.state.catalog.default_model().map(str::to_string))
    }

    fn reload(&mut self) {
        self.conversations = self.core.conversations(0, 100).unwrap_or_default();
        if let Some(c) = &self.current {
            self.current = self.core.conversation(&c.id).ok();
        }
    }

    // ---- actions (buttons, keys and the scenario all come here) ----------------------------

    pub fn show_code(&mut self, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let job = self.core.handle().spawn(async move { core.pair_offer().await });
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let r = job.await;
            let _ = this.update(cx, |v, cx| {
                v.copied = false;
                match r {
                    // Also on stdout, so the code is never stuck in the window.
                    Ok(Ok(link)) => println!("bragi: link code: {link}"),
                    Ok(Err(e)) => v.notice = Some(e.message),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Puts the link code on the clipboard and says so for two seconds.
    pub fn copy_code(&mut self, cx: &mut Context<Self>) {
        let PairState::Offering { link, .. } = &self.state.pair else { return };
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(link.clone()));
        self.copied = true;
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = this.update(cx, |v, cx| {
                v.copied = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// Copies `text` (a code block, or a whole reply as its raw Markdown); its button says
    /// "Copied" for a moment.
    pub fn copy_text(&mut self, id: SharedString, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        self.copied_block = Some(id.clone());
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = this.update(cx, |v, cx| {
                if v.copied_block.as_ref() == Some(&id) {
                    v.copied_block = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// A reply's Markdown, parsed once per text (streaming text is parsed tolerantly).
    fn parsed(&self, key: &str, text: &str, streaming: bool) -> Rc<bragi_core::markdown::Document> {
        let mut cache = self.markdown.borrow_mut();
        if let Some((t, s, doc)) = cache.get(key)
            && t == text
            && *s == streaming
        {
            return doc.clone();
        }
        let doc = Rc::new(if streaming { bragi_core::markdown::parse_streaming(text) } else { bragi_core::markdown::parse(text) });
        cache.insert(key.to_string(), (text.to_string(), streaming, doc.clone()));
        doc
    }

    pub fn confirm_code(&mut self, matches: bool, cx: &mut Context<Self>) {
        if let Err(e) = self.core.pair_confirm(matches) {
            self.notice = Some(e.message);
        }
        cx.notify();
    }

    pub fn new_chat(&mut self, cx: &mut Context<Self>) {
        self.core.dismiss_node_search();
        self.current = None;
        cx.notify();
    }

    pub fn open(&mut self, id: &str, cx: &mut Context<Self>) {
        self.current = self.core.conversation(id).ok();
        self.jump_to_latest();
        cx.notify();
    }

    /// Go to the newest message and follow it again.
    fn jump_to_latest(&mut self) {
        self.stick = true;
        self.scroll.scroll_to_bottom();
    }

    /// New text arrived: stay at the bottom unless the reader scrolled up.
    fn follow_bottom(&mut self) {
        if self.stick {
            self.scroll.scroll_to_bottom();
        }
    }

    fn at_bottom(&self) -> bool {
        self.scroll.max_offset().y <= px(0.) || self.scroll.offset().y <= -self.scroll.max_offset().y + px(4.)
    }

    /// Scroll the list by `dy` (positive moves down); `None` goes to an end.
    fn scroll_by(&mut self, dy: Option<f32>, to_top: bool, cx: &mut Context<Self>) {
        let max = self.scroll.max_offset().y;
        let mut o = self.scroll.offset();
        o.y = match dy {
            Some(d) => (o.y - px(d)).clamp(-max, px(0.)),
            None if to_top => px(0.),
            None => -max,
        };
        self.scroll.set_offset(o);
        self.recheck = true;
        cx.notify();
    }

    fn page(&self) -> f32 {
        (self.scroll.bounds().size.height.as_f32() * 0.9).max(80.)
    }

    /// Put the list where the scrollbar was grabbed.
    fn drag_to(&mut self, y: gpui::Pixels, cx: &mut Context<Self>) {
        let b = self.scroll.bounds();
        let max = self.scroll.max_offset().y;
        if b.size.height <= px(0.) || max <= px(0.) {
            return;
        }
        let frac = ((y - b.origin.y) / b.size.height).clamp(0., 1.);
        self.scroll.set_offset(gpui::point(self.scroll.offset().x, -max * frac));
        self.recheck = true;
        cx.notify();
    }

    pub fn pick_model(&mut self, id: &str, cx: &mut Context<Self>) {
        self.core.dismiss_node_search();
        self.model = Some(id.to_string());
        cx.notify();
    }

    pub fn set_prompt(&mut self, text: &str, cx: &mut Context<Self>) {
        self.prompt.update(cx, |p, cx| p.set_text(text, cx));
    }

    pub fn toggle_attach(&mut self, cx: &mut Context<Self>) {
        self.attach_open = !self.attach_open;
        cx.notify();
    }

    pub fn toggle_agent(&mut self, cx: &mut Context<Self>) {
        self.agent = !self.agent;
        cx.notify();
    }

    /// Adds the file named in the path box to the next message.
    pub fn attach_typed(&mut self, cx: &mut Context<Self>) {
        let path = self.attach_input.read(cx).text().trim().to_string();
        self.attach_path(&path, cx);
    }

    pub fn attach_path(&mut self, path: &str, cx: &mut Context<Self>) {
        let path = match path.strip_prefix("~/") {
            Some(rest) => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)).unwrap_or_else(|| PathBuf::from(path)),
            None => PathBuf::from(path),
        };
        match Attachment::from_path(&path) {
            Ok(a) => {
                self.attachments.push(a);
                self.attach_input.update(cx, |i, cx| i.set_text("", cx));
                self.attach_open = false;
                self.notice = None;
            }
            Err(e) => self.notice = Some(e.message),
        }
        cx.notify();
    }

    pub fn detach(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.attachments.len() {
            self.attachments.remove(index);
        }
        cx.notify();
    }

    pub fn answer_card(&mut self, choice: Choice, cx: &mut Context<Self>) {
        if let Some(c) = self.card.take() {
            self.core.approve(&c.request_id, c.ask, choice);
        }
        cx.notify();
    }

    pub fn send(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.core.dismiss_node_search();
        if self.streaming.is_some() {
            return;
        }
        let text = self.prompt.read(cx).text().to_string();
        let conv = self.current.as_ref().map(|c| c.id.clone());
        let files = std::mem::take(&mut self.attachments);
        let sent = if self.agent {
            self.core.send_agent(conv.as_deref(), &text, None, None, files.clone())
        } else {
            self.core.send_with(conv.as_deref(), &text, self.model(), files.clone())
        };
        match sent {
            Err(e) => {
                self.attachments = files;
                self.notice = Some(e.message);
            }
            Ok(h) => {
                self.prompt.update(cx, |p, cx| p.set_text("", cx));
                self.follow(h, cx);
            }
        }
        cx.notify();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(s) = &self.streaming {
            self.core.cancel(&s.request_id);
        }
        cx.notify();
    }

    pub fn continue_answer(&mut self, cx: &mut Context<Self>) {
        let Some(c) = &self.current else { return };
        match self.core.continue_answer(&c.id, self.model.clone()) {
            Ok(h) => self.follow(h, cx),
            Err(e) => self.notice = Some(e.message),
        }
        cx.notify();
    }

    pub fn link_device(&mut self, cx: &mut Context<Self>) {
        let link = self.link_input.read(cx).text().to_string();
        // A node address (`/ip4/…/tcp/4848/p2p/…`, from `ymirctl pool addresses`) rather than a code:
        // dial that node from now on.
        self.core.dismiss_node_search();
        if bragi_core::is_node_address(&link) {
            // Progress and the outcome come as the core's node search (drawn below the chat, not
            // in red); only a refused address is an error.
            self.notice = self.core.add_node(&link).err().map(|e| e.message);
            self.link_input.update(cx, |i, cx| i.set_text("", cx));
            cx.notify();
            return;
        }
        let core = self.core.clone();
        let job = self.core.handle().spawn(async move { core.pair_accept(&link).await });
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let r = job.await;
            let _ = this.update(cx, |v, cx| {
                match r {
                    Ok(Ok(check)) => v.link_check = Some(check),
                    Ok(Err(e)) => v.notice = Some(e.message),
                    Err(e) => v.notice = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn link_confirm(&mut self, matches: bool, cx: &mut Context<Self>) {
        self.link_check = None;
        let core = self.core.clone();
        let job = self.core.handle().spawn(async move { core.pair_accept_confirm(matches).await });
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            let r = job.await;
            let _ = this.update(cx, |v, cx| {
                v.notice = Some(match r {
                    Ok(Ok(())) if matches => "Linked.".into(),
                    Ok(Ok(())) => "Cancelled.".into(),
                    Ok(Err(e)) => e.message,
                    Err(e) => e.to_string(),
                });
                v.link_input.update(cx, |i, cx| i.set_text("", cx));
                cx.notify();
            });
        })
        .detach();
    }

    fn follow(&mut self, mut h: bragi_core::ChatHandle, cx: &mut Context<Self>) {
        self.streaming = Some(Streaming { request_id: h.request_id.clone(), conversation: h.conversation_id.clone() });
        self.current = self.core.conversation(&h.conversation_id).ok();
        self.reload();
        self.jump_to_latest();
        self.thinking_since = None;
        // Redraws the Thinking section's elapsed time while the model reasons.
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(500)).await;
                let going = this.update(cx, |v, cx| {
                    if v.thinking_since.is_some() {
                        cx.notify();
                    }
                    v.streaming.is_some()
                });
                if !matches!(going, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            while let Some(ev) = h.events.recv().await {
                let end = matches!(ev, ChatEvent::Done { .. } | ChatEvent::Error { .. });
                let alive = this.update(cx, |v, cx| {
                    v.on_chat(&h.request_id, &h.conversation_id, ev);
                    cx.notify();
                });
                if alive.is_err() || end {
                    break;
                }
            }
            let _ = this.update(cx, |v, cx| {
                v.streaming = None;
                v.thinking_since = None;
                v.card = None;
                v.reload();
                v.follow_bottom();
                cx.notify();
            });
        })
        .detach();
    }

    fn on_chat(&mut self, request_id: &str, conversation: &str, ev: ChatEvent) {
        let Some(c) = self.current.as_mut().filter(|c| c.id == conversation) else { return };
        let Some(last) = c.messages.last_mut().filter(|m| m.role == Role::Assistant) else { return };
        match ev {
            ChatEvent::Delta { offset, text } => {
                splice(&mut last.text, offset, &text);
                self.follow_bottom();
            }
            ChatEvent::Reasoning { offset, text } => {
                self.thinking_since.get_or_insert_with(Instant::now);
                splice(&mut last.reasoning, offset, &text);
                self.scroll.scroll_to_bottom();
            }
            ChatEvent::Thought { elapsed_ms } => {
                // A continued answer adds to the time already spent.
                last.thinking_ms = Some(elapsed_ms);
                self.thinking_since = None;
            }
            ChatEvent::Started { node, model, .. } => {
                last.node = Some(node);
                last.model = Some(model);
            }
            ChatEvent::Meta { cache_hit, standin, .. } => {
                last.cache_hit = cache_hit;
                last.standin = standin;
            }
            ChatEvent::Resumed { node, .. } => {
                last.node = Some(node);
                last.resumes += 1;
            }
            ChatEvent::Tool { tool, state: bragi_core::ToolState::Started, .. } => {
                if !last.tools.contains(&tool) {
                    last.tools.push(tool);
                }
            }
            ChatEvent::Approval { ask, summary, tier, .. } => {
                self.card = Some(Card { request_id: request_id.to_string(), ask, summary, tier });
            }
            ChatEvent::Notice { text } => self.notice = Some(text),
            ChatEvent::Retry { attempt, .. } => self.notice = Some(format!("No node answered; trying again ({attempt})…")),
            ChatEvent::Error { kind, message, .. } if kind != bragi_core::ErrorKind::Cancelled => self.notice = Some(message),
            _ => {}
        }
    }

    // ---- drawing ---------------------------------------------------------------------------

    fn title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let control = |id: &'static str, label: &'static str, hover: u32| {
            div()
                .id(id)
                .flex()
                .items_center()
                .justify_center()
                .w(px(32.))
                .h(px(24.))
                .rounded(px(6.))
                .text_color(rgb(t.text_secondary))
                .hover(move |s| s.bg(rgb(hover)).text_color(rgb(t.text_primary)))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(label)
        };
        let mut controls = div().flex().flex_row().gap(px(2.));
        if !self.close_only {
            controls = controls
                .child(control("min", "—", t.surface_variant).on_click(|_, window, _| window.minimize_window()))
                .child(control("max", "□", t.surface_variant).on_click(|_, window, _| window.zoom_window()));
        }
        controls = controls.child(control("close", "×", t.error).on_click(cx.listener(|_, _, _, cx| cx.quit())));
        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(36.))
            .px(px(12.))
            .bg(rgb(t.surface))
            .border_b_1()
            .border_color(rgb(t.border))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .child(
                div().flex().flex_row().gap(px(8.)).items_center().child(div().text_color(rgb(t.accent)).text_size(px(15.)).child("ᛒ Bragi")).child(
                    div().text_color(rgb(t.text_secondary)).text_size(px(12.)).child(match &self.state.pair {
                        PairState::Linked { device, .. } => format!("{device} · on your Weirdnet"),
                        _ => "not linked".to_string(),
                    }),
                ),
            )
            .child(controls)
    }

    fn button(&self, id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, primary: bool) -> gpui::Stateful<gpui::Div> {
        let t = self.theme;
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .px(px(14.))
            .h(px(32.))
            .rounded(px(8.))
            .text_size(px(13.))
            .bg(rgb(if primary { t.accent } else { t.surface_variant }))
            .text_color(rgb(if primary { t.background } else { t.text_primary }))
            .cursor_pointer()
            .hover(|s| s.opacity(0.85))
            .child(label.into())
    }

    fn qr(&self, link: &str) -> impl IntoElement {
        let mut grid = div().flex().flex_col().p(px(10.)).bg(rgb(0xFFFFFF)).rounded(px(8.));
        if let Ok(code) = qrcode::QrCode::new(link.as_bytes()) {
            let w = code.width();
            let colors = code.to_colors();
            let cell = (220.0 / w as f32).floor().max(2.0);
            for row in colors.chunks(w) {
                let mut r = div().flex().flex_row();
                for c in row {
                    let dark = *c == qrcode::Color::Dark;
                    r = r.child(div().w(px(cell)).h(px(cell)).bg(rgb(if dark { 0x000000 } else { 0xFFFFFF })));
                }
                grid = grid.child(r);
            }
        }
        grid
    }

    fn pairing(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let card = div().flex().flex_col().items_center().gap(px(14.)).p(px(28.)).w(px(520.)).rounded(px(14.)).bg(rgb(t.surface));
        let title = |s: &str| div().text_size(px(20.)).text_color(rgb(t.text_primary)).child(s.to_string());
        let note = |s: String| div().text_size(px(13.)).text_color(rgb(t.text_secondary)).text_center().child(s);
        let card = match &self.state.pair {
            PairState::Unpaired | PairState::Failed { .. } => {
                let mut c = card
                    .child(title("Link Bragi to your Weirdnet"))
                    .child(note("Show a code here, then open it on a device that is already linked: Morph, Ymir or another Bragi.".into()));
                if let PairState::Failed { message } = &self.state.pair {
                    c = c.child(div().text_size(px(13.)).text_color(rgb(t.error)).child(message.clone()));
                }
                c.child(self.button("show-code", "Show a link code", true).on_click(cx.listener(|v, _, _, cx| v.show_code(cx))))
            }
            PairState::Offering { link, .. } => card
                .child(title("Open this code on your other device"))
                .child(self.qr(link))
                .child(div().id("link-code").w_full().text_size(px(10.)).text_color(rgb(t.text_secondary)).child(link.clone()))
                .child(
                    self.button("copy-code", if self.copied { "Copied" } else { "Copy code" }, true)
                        .on_click(cx.listener(|v, _, _, cx| v.copy_code(cx))),
                )
                .child(note("It works for two minutes.".into())),
            PairState::Confirm { sas } => card
                .child(title("Do both screens show this code?"))
                .child(div().text_size(px(40.)).text_color(rgb(t.accent)).child(format!("{} {}", &sas[..3], &sas[3..])))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(10.))
                        .child(self.button("codes-match", "They match", true).on_click(cx.listener(|v, _, _, cx| v.confirm_code(true, cx))))
                        .child(self.button("codes-differ", "They don't", false).on_click(cx.listener(|v, _, _, cx| v.confirm_code(false, cx)))),
                ),
            PairState::Waiting { sas } => card.child(title("Confirm on the other device")).child(note(format!("Code {sas}. Waiting for it…"))),
            PairState::Linked { .. } => card.child(title("Linked")),
        };
        div().flex().flex_1().items_center().justify_center().child(card)
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let mut list = div().id("conversations").flex().flex_col().gap(px(2.)).flex_1().overflow_y_scroll();
        for c in &self.conversations {
            let selected = self.current.as_ref().is_some_and(|x| x.id == c.id);
            let id = c.id.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("conv-{}", c.id)))
                    .px(px(10.))
                    .py(px(7.))
                    .rounded(px(6.))
                    .text_size(px(13.))
                    .text_color(rgb(if selected { t.text_primary } else { t.text_secondary }))
                    .when(selected, |d| d.bg(rgb(t.surface_variant)))
                    .hover(|s| s.bg(rgb(t.surface_variant)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |v, _, _, cx| v.open(&id, cx)))
                    .child(c.title.clone()),
            );
        }
        let link_box = match &self.link_check {
            Some(check) => div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(div().text_size(px(12.)).text_color(rgb(t.text_secondary)).child(format!("{} shows {}?", check.device, check.sas)))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(6.))
                        .child(self.button("link-yes", "Match", true).on_click(cx.listener(|v, _, _, cx| v.link_confirm(true, cx))))
                        .child(self.button("link-no", "Cancel", false).on_click(cx.listener(|v, _, _, cx| v.link_confirm(false, cx)))),
                ),
            None => div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child("LINK A DEVICE"))
                .child(
                    div()
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(px(6.))
                        .bg(rgb(t.background))
                        .text_size(px(12.))
                        .overflow_hidden()
                        .child(self.link_input.clone()),
                )
                .child(self.button("link-go", "Link", false).on_click(cx.listener(|v, _, _, cx| v.link_device(cx)))),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .flex_none()
            .w(px(240.))
            .p(px(10.))
            .bg(rgb(t.surface))
            .border_r_1()
            .border_color(rgb(t.border))
            .child(self.button("new-chat", "+ New chat", false).on_click(cx.listener(|v, _, _, cx| v.new_chat(cx))))
            .child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child("HISTORY"))
            .child(list)
            .child(link_box)
    }

    fn models(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let picked = self.model();
        let mut row = div()
            .id("models")
            .flex()
            .flex_none()
            .flex_row()
            .gap(px(8.))
            .items_center()
            .px(px(16.))
            .py(px(10.))
            .overflow_x_scroll()
            .border_b_1()
            .border_color(rgb(t.border));
        if self.state.catalog.models.is_empty() {
            row = row.child(div().text_size(px(13.)).text_color(rgb(t.text_secondary)).child("Looking for your inference nodes…"));
        }
        for m in &self.state.catalog.models {
            let selected = picked.as_deref() == Some(m.id.as_str());
            let (dot, label) = match m.status {
                ModelStatus::Warm => (0x4CD07D, "warm"),
                ModelStatus::Loading => (t.accent, "loading"),
                ModelStatus::Busy => (0xFF9F43, "busy"),
                ModelStatus::Loadable => (t.text_secondary, "loads on use"),
            };
            let speed = m.tokens_per_sec.map(|s| format!(" · {s:.0} tok/s")).unwrap_or_default();
            let on = if self.state.catalog.nodes.len() > 1 {
                format!(" · {} of {} nodes", m.nodes.len(), self.state.catalog.nodes.len())
            } else {
                String::new()
            };
            let id = m.id.clone();
            row = row.child(
                div()
                    .id(SharedString::from(format!("model-{}", m.id)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .flex_none()
                    .px(px(10.))
                    .py(px(5.))
                    .rounded(px(14.))
                    .border_1()
                    .border_color(rgb(if selected { t.accent } else { t.border }))
                    .bg(rgb(if selected { t.surface_variant } else { t.surface }))
                    .cursor_pointer()
                    .on_click(cx.listener(move |v, _, _, cx| v.pick_model(&id, cx)))
                    .child(div().w(px(8.)).h(px(8.)).rounded(px(4.)).bg(rgb(dot)))
                    .child(div().text_size(px(13.)).text_color(rgb(t.text_primary)).child(m.id.clone()))
                    .child(div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child(format!("{label}{speed}{on}"))),
            );
        }
        row
    }

    /// The collapsible Thinking section above an answer: the model's reasoning as plain text,
    /// streaming live, with the time it took. Open while the model reasons, closed once the
    /// answer starts; a click flips it.
    fn thinking(&self, index: usize, m: &ChatMessage, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if m.role != Role::Assistant || m.reasoning.trim().is_empty() {
            return None;
        }
        let t = self.theme;
        let streaming = m.state == MessageState::Streaming;
        let reasoning_now = streaming && m.text.is_empty() && m.thinking_ms.is_none();
        let conversation = self.current.as_ref().map(|c| c.id.clone()).unwrap_or_default();
        let key = (conversation, index);
        let open = reasoning_now != self.thinking_flipped.contains(&key);
        let elapsed = match (reasoning_now, self.thinking_since) {
            (true, Some(since)) => since.elapsed().as_millis() as u64,
            _ => m.thinking_ms.unwrap_or(0),
        };
        let title = format!("{} {}", if open { "▾" } else { "▸" }, bragi_core::think::label(elapsed, !reasoning_now));
        let header = div()
            .id(SharedString::from(format!("thinking-{index}")))
            .text_size(px(12.))
            .text_color(rgb(t.text_secondary))
            .cursor_pointer()
            .hover(|s| s.opacity(0.8))
            .child(title)
            .on_click(cx.listener(move |v, _, _, cx| {
                if !v.thinking_flipped.remove(&key) {
                    v.thinking_flipped.insert(key.clone());
                }
                cx.notify();
            }));
        Some(div().flex().flex_col().gap(px(4.)).child(header).when(open, |d| {
            d.child(
                div()
                    .pl(px(10.))
                    .border_l_2()
                    .border_color(rgb(t.border))
                    .text_size(px(13.))
                    .line_height(px(19.))
                    .text_color(rgb(t.text_secondary))
                    .child(m.reasoning.trim().to_string()),
            )
        }))
    }

    fn bubble(&self, i: usize, m: &ChatMessage, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let mine = m.role == Role::User;
        let meta = match (m.role, m.state) {
            (Role::User, _) => None,
            (_, MessageState::Streaming) => Some(format!("{} · answering…", m.model.clone().unwrap_or_default())),
            (_, MessageState::Stopped) => Some(format!("{} · stopped", m.model.clone().unwrap_or_default())),
            (_, MessageState::Failed) => Some(format!("{} · cut off", m.model.clone().unwrap_or_default())),
            (_, MessageState::Complete) => m.model.clone(),
        };
        // What the pool did: served from the cache, answered by a stand-in, moved between nodes.
        let flags: Vec<String> = [
            m.cache_hit.then(|| "cached".to_string()),
            m.standin.as_ref().map(|s| format!("stand-in {s}")),
            (m.resumes > 0).then(|| if m.resumes == 1 { "moved to another node".to_string() } else { format!("moved nodes {}×", m.resumes) }),
            m.agent.then(|| if m.tools.is_empty() { "agent task".to_string() } else { format!("agent task · used {}", m.tools.join(", ")) }),
        ]
        .into_iter()
        .flatten()
        .collect();
        let meta = match (meta, flags.is_empty()) {
            (Some(s), false) => Some(format!("{s} · {}", flags.join(" · "))),
            (None, false) if m.role == Role::Assistant => Some(flags.join(" · ")),
            (meta, _) => meta,
        };
        div().flex().flex_row().when(mine, |d| d.justify_end()).child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .max_w(px(620.))
                .when(!mine, |d| d.w_full())
                .px(px(14.))
                .py(px(10.))
                .rounded(px(12.))
                .bg(rgb(if mine { t.selection } else { t.surface }))
                .children(meta.map(|s| div().text_size(px(11.)).text_color(rgb(t.text_secondary)).child(s)))
                .children(m.attachments.iter().map(|a| {
                    div()
                        .px(px(8.))
                        .py(px(3.))
                        .rounded(px(6.))
                        .bg(rgb(t.surface_variant))
                        .text_size(px(12.))
                        .text_color(rgb(t.text_secondary))
                        .child(format!("attached: {} ({} lines)", a.name, a.text.lines().count()))
                }))
                .children(self.thinking(i, m, cx))
                .child(self.message_text(i, m, cx)),
        )
    }

    /// The message list, filling the space between the header and the composer: a scrollbar on
    /// its right edge and a "jump to latest" button when the reader is above the newest text.
    fn message_area(&self, body: impl IntoElement, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let max = self.scroll.max_offset().y.as_f32();
        let view = self.scroll.bounds().size.height.as_f32();
        let mut area = div()
            .id("message-area")
            .relative()
            .flex_1()
            .min_h(px(0.))
            .overflow_hidden()
            .on_mouse_move(cx.listener(|v, e: &gpui::MouseMoveEvent, _, cx| {
                if v.dragging && e.pressed_button == Some(MouseButton::Left) {
                    v.drag_to(e.position.y, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|v, _, _, _| {
                    v.dragging = false;
                }),
            )
            .child(body);
        if max > 0. && view > 0. {
            let total = max + view;
            let thumb = (view * view / total).max(28.);
            let top = (-self.scroll.offset().y.as_f32() / max).clamp(0., 1.) * (view - thumb);
            area = area.child(
                div()
                    .id("scrollbar")
                    .absolute()
                    .top(px(0.))
                    .bottom(px(0.))
                    .right(px(0.))
                    .w(px(12.))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|v, e: &gpui::MouseDownEvent, _, cx| {
                            v.dragging = true;
                            v.drag_to(e.position.y, cx);
                        }),
                    )
                    .child(div().absolute().top(px(top)).right(px(3.)).w(px(6.)).h(px(thumb)).rounded(px(3.)).bg(rgb(t.text_secondary)).opacity(0.5)),
            );
        }
        if !self.stick && max > 0. {
            area = area.child(div().absolute().bottom(px(12.)).left(px(0.)).right(px(0.)).flex().justify_center().child(
                self.button("jump-latest", "↓ Jump to latest", true).on_click(cx.listener(|v, _, _, cx| {
                    v.jump_to_latest();
                    cx.notify();
                })),
            ));
        }
        area
    }

    /// A user's message as typed; a reply as Markdown, with a Copy button for its raw text.
    fn message_text(&self, i: usize, m: &ChatMessage, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = self.theme;
        let text = div().text_size(px(14.)).line_height(px(21.)).text_color(rgb(t.text_primary));
        if m.text.is_empty() {
            // Nothing from the model yet: say so, until reasoning or text arrives.
            let waiting = m.state == MessageState::Streaming && m.reasoning.trim().is_empty();
            return text.child(if waiting { "Waiting for a reply…" } else { "…" }).into_any_element();
        }
        if m.role == Role::User {
            return text.child(m.text.clone()).into_any_element();
        }
        let conv = self.current.as_ref().map(|c| c.id.as_str()).unwrap_or("");
        let key = format!("{conv}-{i}");
        let doc = self.parsed(&key, &m.text, m.state == MessageState::Streaming);
        let weak = cx.entity().downgrade();
        let on_copy: markdown::OnCopy = Rc::new(move |id, text, cx| {
            let _ = weak.update(cx, |v, cx| v.copy_text(id, text, cx));
        });
        let style = markdown::Style { theme: t, mono: self.mono.clone(), copied: self.copied_block.clone(), on_copy };
        let copy_id = SharedString::from(format!("msg-{key}"));
        let copied = self.copied_block.as_ref() == Some(&copy_id);
        let raw = m.text.clone();
        let copy = div()
            .id(SharedString::from(format!("copy-{key}")))
            .px(px(8.))
            .py(px(2.))
            .rounded(px(5.))
            .text_size(px(11.))
            .text_color(rgb(t.text_secondary))
            .cursor_pointer()
            .hover(|h| h.bg(rgb(t.surface_variant)).text_color(rgb(t.text_primary)))
            .on_click(cx.listener(move |v, _, _, cx| v.copy_text(copy_id.clone(), raw.clone(), cx)))
            .child(if copied { "Copied" } else { "Copy" });
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(text.child(markdown::render(&doc, &style, &key)))
            .when(m.state != MessageState::Streaming, |d| d.child(div().flex().flex_row().justify_end().child(copy)))
            .into_any_element()
    }

    fn chat(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let mut body =
            div().id("messages").track_scroll(&self.scroll).flex().flex_col().gap(px(12.)).p(px(16.)).size_full().overflow_y_scroll().on_scroll(
                cx.listener(|v, _, _, cx| {
                    v.recheck = true;
                    cx.notify();
                }),
            );
        match &self.current {
            Some(c) => {
                for (i, m) in c.messages.iter().enumerate() {
                    body = body.child(self.bubble(i, m, cx));
                }
            }
            None => {
                body = body.child(
                    div().flex().flex_1().items_center().justify_center().text_color(rgb(t.text_secondary)).child("Ask your own cloud something."),
                )
            }
        }
        let streaming = self.streaming.as_ref().is_some_and(|s| self.current.as_ref().is_some_and(|c| c.id == s.conversation));
        let can_continue = self.streaming.is_none() && self.current.as_ref().is_some_and(|c| c.continuable());
        let mut bar = div()
            .flex()
            .flex_row()
            .gap(px(8.))
            .items_center()
            .p(px(12.))
            .child(self.button("attach", "Attach", false).on_click(cx.listener(|v, _, _, cx| v.toggle_attach(cx))))
            .child(
                self.button("agent", if self.agent { "Agent: on" } else { "Agent: off" }, self.agent)
                    .on_click(cx.listener(|v, _, _, cx| v.toggle_agent(cx))),
            )
            .child(div().flex_1().px(px(12.)).py(px(9.)).rounded(px(8.)).bg(rgb(t.surface)).text_size(px(14.)).child(self.prompt.clone()));
        if can_continue {
            bar = bar.child(self.button("continue", "Continue", false).on_click(cx.listener(|v, _, _, cx| v.continue_answer(cx))));
        }
        bar = if streaming {
            bar.child(self.button("stop", "■ Stop", false).on_click(cx.listener(|v, _, _, cx| v.stop(cx))))
        } else {
            bar.child(self.button("send", "Send", true).on_click(cx.listener(|v, _, window, cx| v.send(window, cx))))
        };
        // Files chosen for the next message, the path box, and an agent task's approval card.
        let mut above = div().flex().flex_col().gap(px(8.)).px(px(12.)).border_t_1().border_color(rgb(t.border));
        let mut any = false;
        if !self.attachments.is_empty() {
            any = true;
            let mut chips = div().flex().flex_row().gap(px(6.)).pt(px(8.));
            for (i, a) in self.attachments.iter().enumerate() {
                chips = chips.child(
                    div()
                        .id(SharedString::from(format!("chip-{i}")))
                        .px(px(10.))
                        .py(px(4.))
                        .rounded(px(12.))
                        .bg(rgb(t.surface_variant))
                        .text_size(px(12.))
                        .cursor_pointer()
                        .on_click(cx.listener(move |v, _, _, cx| v.detach(i, cx)))
                        .child(format!("{} ({} lines) ×", a.name, a.text.lines().count())),
                );
            }
            above = above.child(chips);
        }
        if self.attach_open {
            any = true;
            above = above.child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.))
                    .items_center()
                    .pt(px(8.))
                    .child(
                        div().flex_1().px(px(12.)).py(px(8.)).rounded(px(8.)).bg(rgb(t.surface)).text_size(px(13.)).child(self.attach_input.clone()),
                    )
                    .child(self.button("attach-add", "Add file", true).on_click(cx.listener(|v, _, _, cx| v.attach_typed(cx)))),
            );
        }
        if let Some(card) = &self.card {
            any = true;
            above = above.child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(10.))
                    .items_center()
                    .mt(px(8.))
                    .p(px(10.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgb(t.accent))
                    .bg(rgb(t.surface))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(div().text_size(px(13.)).text_color(rgb(t.text_primary)).child(card.summary.clone()))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(t.text_secondary))
                                    .child(format!("the node asks to {} · only you can say yes", card.tier)),
                            ),
                    )
                    .child(self.button("allow-once", "Allow once", true).on_click(cx.listener(|v, _, _, cx| v.answer_card(Choice::Once, cx))))
                    .child(
                        self.button("allow-task", "Allow for this task", false)
                            .on_click(cx.listener(|v, _, _, cx| v.answer_card(Choice::ThisTask, cx))),
                    )
                    .child(self.button("deny", "Deny", false).on_click(cx.listener(|v, _, _, cx| v.answer_card(Choice::Deny, cx)))),
            );
        }
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .overflow_hidden()
            .child(self.models(cx))
            .child(self.message_area(body, cx))
            .children(self.notice.clone().map(|n| div().px(px(16.)).py(px(6.)).text_size(px(12.)).text_color(rgb(t.error)).child(n)))
            .children(self.state.node_search.line().map(|line| {
                let color = match &self.state.node_search {
                    NodeSearch::Found { .. } => t.success,
                    s if s.is_problem() => t.warning,
                    _ => t.text_secondary,
                };
                div().px(px(16.)).py(px(6.)).text_size(px(12.)).text_color(rgb(color)).child(line)
            }))
            .children(any.then_some(above.flex_none().max_h(px(220.)).overflow_hidden()))
            .child(div().flex_none().border_t_1().border_color(rgb(t.border)).child(bar))
    }
}

impl Render for BragiView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        if std::mem::take(&mut self.recheck) {
            self.stick = self.at_bottom();
        }
        let linked = matches!(self.state.pair, PairState::Linked { .. });
        let main = if linked {
            div().flex().flex_row().flex_1().min_h(px(0.)).child(self.sidebar(cx)).child(self.chat(cx))
        } else {
            div().flex().flex_1().min_h(px(0.)).child(self.pairing(cx))
        };
        div()
            .key_context("Bragi")
            .on_action(cx.listener(|v, _: &Stop, _, cx| v.stop(cx)))
            .on_action(cx.listener(|_, _: &Quit, _, cx| cx.quit()))
            .on_action(cx.listener(|v, _: &ChatPageUp, _, cx| {
                let d = v.page();
                v.scroll_by(Some(-d), false, cx)
            }))
            .on_action(cx.listener(|v, _: &ChatPageDown, _, cx| {
                let d = v.page();
                v.scroll_by(Some(d), false, cx)
            }))
            .on_action(cx.listener(|v, _: &ChatTop, _, cx| v.scroll_by(None, true, cx)))
            .on_action(cx.listener(|v, _: &ChatBottom, _, cx| v.scroll_by(None, false, cx)))
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(t.background))
            .text_color(rgb(t.text_primary))
            .child(self.title_bar(cx))
            .child(main)
    }
}

// ---- the shared scenario ---------------------------------------------------------------------

async fn wait_for(this: &WeakEntity<BragiView>, cx: &mut gpui::AsyncApp, what: &str, f: impl Fn(&BragiView) -> bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let ok = this.read_with(cx, |v, _| f(v)).map_err(|e| e.to_string())?;
        if ok {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!("timed out waiting for {what}"));
        }
        cx.background_executor().timer(Duration::from_millis(50)).await;
    }
}

/// The demo nodes' scripted trouble, for the scenario.
static FAULTS: std::sync::OnceLock<Arc<Faults>> = std::sync::OnceLock::new();
static DEMO: std::sync::OnceLock<Arc<DemoNode>> = std::sync::OnceLock::new();

fn last_assistant(v: &BragiView) -> Option<ChatMessage> {
    v.current.as_ref().and_then(|c| c.messages.last().cloned()).filter(|m| m.role == Role::Assistant)
}

/// Pair, list models, chat, cancel, continue: the same steps the Flutter app's test plays.
async fn scenario(this: WeakEntity<BragiView>, cx: &mut gpui::AsyncApp) -> Result<(), String> {
    let pause = Duration::from_millis(std::env::var("BRAGI_SCENARIO_PAUSE_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(800));
    let step = |name: &str| println!("bragi-scenario: {name}");
    let shot = async |cx: &mut gpui::AsyncApp, name: &str| {
        println!("bragi-scenario: screenshot {name}");
        cx.background_executor().timer(pause).await;
    };
    step("start");
    shot(cx, "01-unpaired").await;
    this.update(cx, |v, cx| v.show_code(cx)).map_err(|e| e.to_string())?;
    wait_for(&this, cx, "the code", |v| matches!(v.state.pair, PairState::Offering { .. } | PairState::Confirm { .. })).await?;
    // Copy puts the offered code on the clipboard (when the demo peer has not already moved on).
    let copied = this
        .update(cx, |v, cx| {
            let PairState::Offering { link, .. } = v.state.pair.clone() else { return None };
            v.copy_code(cx);
            Some((link, cx.read_from_clipboard().and_then(|i| i.text())))
        })
        .map_err(|e| e.to_string())?;
    if let Some((link, got)) = copied {
        if got.as_deref() != Some(link.as_str()) {
            return Err("the Copy button did not put the link code on the clipboard".into());
        }
        step("copied the link code");
    }
    shot(cx, "02-code").await;
    wait_for(&this, cx, "the check", |v| matches!(v.state.pair, PairState::Confirm { .. })).await?;
    shot(cx, "03-check").await;
    this.update(cx, |v, cx| v.confirm_code(true, cx)).map_err(|e| e.to_string())?;
    wait_for(&this, cx, "linked", |v| matches!(v.state.pair, PairState::Linked { .. })).await?;
    step("paired");
    wait_for(&this, cx, "the models", |v| v.state.catalog.models.len() >= 2).await?;
    step("models listed");
    shot(cx, "04-models").await;
    this.update(cx, |v, cx| v.set_prompt("What does Bragi do?", cx)).map_err(|e| e.to_string())?;
    press_send(&this, cx)?;
    wait_for(&this, cx, "the answer", |v| v.streaming.is_none() && last_assistant(v).is_some_and(|m| m.state == MessageState::Complete)).await?;
    step("chatted");
    shot(cx, "05-answer").await;
    this.update(cx, |v, cx| v.set_prompt("Tell me more about cancel.", cx)).map_err(|e| e.to_string())?;
    press_send(&this, cx)?;
    wait_for(&this, cx, "some text", |v| last_assistant(v).is_some_and(|m| m.text.split(' ').count() > 8)).await?;
    shot(cx, "06-streaming").await;
    this.update(cx, |v, cx| v.stop(cx)).map_err(|e| e.to_string())?;
    wait_for(&this, cx, "stopped", |v| v.streaming.is_none() && last_assistant(v).is_some_and(|m| m.state == MessageState::Stopped)).await?;
    step("cancelled");
    shot(cx, "07-stopped").await;
    let partial = this.read_with(cx, |v, _| last_assistant(v).map(|m| m.text).unwrap_or_default()).map_err(|e| e.to_string())?;
    this.update(cx, |v, cx| v.continue_answer(cx)).map_err(|e| e.to_string())?;
    wait_for(&this, cx, "the rest", |v| v.streaming.is_none() && last_assistant(v).is_some_and(|m| m.state == MessageState::Complete)).await?;
    let full = this.read_with(cx, |v, _| last_assistant(v).map(|m| m.text).unwrap_or_default()).map_err(|e| e.to_string())?;
    let model = this.read_with(cx, |v, _| last_assistant(v).and_then(|m| m.model)).map_err(|e| e.to_string())?.unwrap_or_default();
    if !(full.starts_with(&partial) && full == bragi_core::demo::answer_for(&model, "Tell me more about cancel.")) {
        return Err(format!("the continued answer is wrong: {full:?}"));
    }
    step("continued");
    shot(cx, "08-continued").await;

    // B2: a model that is not warm anywhere (a stand-in answers), the same question twice (the
    // second is a cache hit), and a node dying mid-answer (another finishes it).
    let faults = FAULTS.get().ok_or("no demo nodes")?.clone();
    this.update(cx, |v, cx| {
        v.new_chat(cx);
        v.pick_model(bragi_core::demo::COLD_MODEL, cx);
        v.set_prompt("Why is the first answer from a stand-in?", cx)
    })
    .map_err(|e| e.to_string())?;
    press_send(&this, cx)?;
    wait_for(&this, cx, "the stand-in's answer", |v| v.streaming.is_none() && last_assistant(v).is_some_and(|m| m.state == MessageState::Complete))
        .await?;
    if this.read_with(cx, |v, _| last_assistant(v).and_then(|m| m.standin)).map_err(|e| e.to_string())?.is_none() {
        return Err("the stand-in's answer is not flagged".into());
    }
    step("stand-in");
    shot(cx, "09-standin").await;
    this.update(cx, |v, cx| {
        v.new_chat(cx);
        v.pick_model(bragi_core::demo::WARM_MODEL, cx);
        v.set_prompt("What does a pool of nodes give me?", cx)
    })
    .map_err(|e| e.to_string())?;
    for round in 0..2 {
        press_send(&this, cx)?;
        wait_for(&this, cx, "the answer", |v| v.streaming.is_none() && last_assistant(v).is_some_and(|m| m.state == MessageState::Complete)).await?;
        if round == 0 {
            this.update(cx, |v, cx| v.set_prompt("What does a pool of nodes give me?", cx)).map_err(|e| e.to_string())?;
        }
    }
    if !this.read_with(cx, |v, _| last_assistant(v).is_some_and(|m| m.cache_hit)).map_err(|e| e.to_string())? {
        return Err("the repeated question is not a cache hit".into());
    }
    step("cache hit");
    shot(cx, "10-cached").await;
    faults.die_next_answer_after(14);
    this.update(cx, |v, cx| {
        v.new_chat(cx);
        v.set_prompt("Keep going even if a node dies.", cx)
    })
    .map_err(|e| e.to_string())?;
    press_send(&this, cx)?;
    wait_for(&this, cx, "the answer", |v| v.streaming.is_none() && last_assistant(v).is_some_and(|m| m.state == MessageState::Complete)).await?;
    let (text, moved) = this.read_with(cx, |v, _| last_assistant(v).map(|m| (m.text, m.resumes)).unwrap_or_default()).map_err(|e| e.to_string())?;
    if moved != 1 || text != bragi_core::demo::answer_for(bragi_core::demo::WARM_MODEL, "Keep going even if a node dies.") {
        return Err(format!("failover: moved {moved} times, text {text:?}"));
    }
    step("failover");
    shot(cx, "11-failover").await;

    // B4: the user's other device (a second core, linked to the same demo identity) starts a
    // conversation; after a sync pass it is in this window's list, and can be opened and read.
    let demo = DEMO.get().ok_or("no demo node")?.clone();
    let core = this.read_with(cx, |v, _| v.core.clone()).map_err(|e| e.to_string())?;
    let handle = core.handle().clone();
    let job = handle.spawn(async move { demo.play_other_device(&core).await });
    let id = job.await.map_err(|e| e.to_string())?.map_err(|e| e.message)?;
    wait_for(&this, cx, "the other device's conversation in the list", |v| v.conversations.iter().any(|c| c.id == id)).await?;
    step("history from another device");
    shot(cx, "12-other-device").await;
    this.update(cx, |v, cx| v.open(&id, cx)).map_err(|e| e.to_string())?;
    wait_for(&this, cx, "it opened", |v| v.current.as_ref().is_some_and(|c| c.id == id && c.messages.len() == 2)).await?;
    shot(cx, "13-other-device-opened").await;

    // B5: a text file attached to a question (the model reads it inline), then agent mode: a task
    // on the node asks to use a tool, the user allows it once, and the answer says what it used.
    let notes = std::env::temp_dir().join(format!("bragi-scenario-notes-{}.md", std::process::id()));
    std::fs::write(&notes, "# Notes\n- buy milk\n- write the B5 milestone\n").map_err(|e| e.to_string())?;
    this.update(cx, |v, cx| {
        v.new_chat(cx);
        v.pick_model(bragi_core::demo::WARM_MODEL, cx);
        v.toggle_attach(cx);
        v.attach_input.update(cx, |i, cx| i.set_text(&notes.display().to_string(), cx));
    })
    .map_err(|e| e.to_string())?;
    shot(cx, "14-attach-path").await;
    this.update(cx, |v, cx| {
        v.attach_typed(cx);
        v.set_prompt("What is on my list?", cx)
    })
    .map_err(|e| e.to_string())?;
    wait_for(&this, cx, "the file chip", |v| v.attachments.len() == 1).await?;
    shot(cx, "15-attached").await;
    press_send(&this, cx)?;
    wait_for(&this, cx, "the answer", |v| v.streaming.is_none() && last_assistant(v).is_some_and(|m| m.state == MessageState::Complete)).await?;
    let text = this.read_with(cx, |v, _| last_assistant(v).map(|m| m.text).unwrap_or_default()).map_err(|e| e.to_string())?;
    if !text.contains("I read bragi-scenario-notes-") || !text.contains("(3 lines)") {
        return Err(format!("the attachment did not reach the model: {text:?}"));
    }
    step("attachment");
    shot(cx, "16-attachment-answer").await;
    let _ = std::fs::remove_file(&notes);

    this.update(cx, |v, cx| {
        v.new_chat(cx);
        v.toggle_agent(cx);
        v.set_prompt("Find my notes about groceries.", cx)
    })
    .map_err(|e| e.to_string())?;
    press_send(&this, cx)?;
    wait_for(&this, cx, "the approval card", |v| v.card.is_some()).await?;
    step("approval card");
    shot(cx, "17-approval").await;
    this.update(cx, |v, cx| v.answer_card(bragi_core::Choice::Once, cx)).map_err(|e| e.to_string())?;
    wait_for(&this, cx, "the task", |v| v.streaming.is_none() && last_assistant(v).is_some_and(|m| m.state == MessageState::Complete)).await?;
    let (text, tools) = this.read_with(cx, |v, _| last_assistant(v).map(|m| (m.text, m.tools)).unwrap_or_default()).map_err(|e| e.to_string())?;
    if !text.contains("2 notes mention") || tools != ["notes.search"] {
        return Err(format!("agent task: {text:?} {tools:?}"));
    }
    step("agent task");
    shot(cx, "18-agent-answer").await;

    // A node address pasted into Link a device: the placeholder fits the field, progress is
    // not red, a node that never answers turns into advice, and a found one says so and clears.
    shot(cx, "19-link-field").await;
    let demo = DEMO.get().ok_or("no demo node")?.clone();
    let second = demo.siblings().first().cloned().ok_or("no second demo node")?;
    let gone = second.address();
    second.stop();
    wait_for(&this, cx, "the stopped node gone", |v| v.state.catalog.nodes.len() == 1).await?;
    this.update(cx, |v, cx| {
        v.core.set_node_search_limit(Duration::from_secs(4));
        v.link_input.update(cx, |i, cx| i.set_text(&gone, cx));
        v.link_device(cx);
    })
    .map_err(|e| e.to_string())?;
    wait_for(&this, cx, "looking", |v| matches!(v.state.node_search, NodeSearch::Looking { .. }) && v.notice.is_none()).await?;
    step("looking for the node");
    shot(cx, "20-node-looking").await;
    wait_for(&this, cx, "the advice", |v| v.state.node_search.is_problem()).await?;
    step("node not found: advice");
    shot(cx, "21-node-not-found").await;
    let first = demo.address();
    this.update(cx, |v, cx| {
        v.link_input.update(cx, |i, cx| i.set_text(&first, cx));
        v.link_device(cx);
    })
    .map_err(|e| e.to_string())?;
    wait_for(&this, cx, "connected", |v| matches!(v.state.node_search, NodeSearch::Found { .. })).await?;
    step("node found");
    shot(cx, "22-node-found").await;
    wait_for(&this, cx, "the line cleared", |v| v.state.node_search == NodeSearch::Idle).await?;
    step("node line cleared");
    shot(cx, "23-node-cleared").await;
    // A long chat: it opens at the newest message, then the reader scrolls to the middle; the
    // composer stays put and "jump to latest" shows.
    this.update(cx, |v, cx| {
        if let Some(c) = v.current.as_mut() {
            let seed = c.messages.clone();
            for i in 0..40 {
                let mut m = seed[i % seed.len()].clone();
                m.text = format!("Message {i}. {}", m.text);
                c.messages.push(m);
            }
        }
        v.jump_to_latest();
        cx.notify();
    })
    .map_err(|e| e.to_string())?;
    cx.background_executor().timer(Duration::from_millis(300)).await;
    this.update(cx, |v, cx| {
        let half = v.scroll.max_offset().y.as_f32() / 2.;
        v.scroll_by(Some(-half), false, cx);
    })
    .map_err(|e| e.to_string())?;
    cx.background_executor().timer(Duration::from_millis(300)).await;
    let (stuck, max) = this.read_with(cx, |v, _| (v.stick, v.scroll.max_offset().y.as_f32())).map_err(|e| e.to_string())?;
    if stuck || max <= 0. {
        return Err(format!("the long chat did not scroll up (stick {stuck}, max {max})"));
    }
    step("scrolled to the middle");
    shot(cx, "24-scrolled-middle").await;
    // Markdown: a reply with a heading, a list, a table and a code block renders as such while it
    // streams (a half-open fence is code already) and when done; Copy gives the raw Markdown.
    this.update(cx, |v, cx| {
        v.new_chat(cx);
        if v.agent {
            v.toggle_agent(cx);
        }
        v.pick_model(bragi_core::demo::WARM_MODEL, cx);
        v.set_prompt("Show me some Markdown.", cx)
    })
    .map_err(|e| e.to_string())?;
    press_send(&this, cx)?;
    wait_for(&this, cx, "a half-open fence", |v| last_assistant(v).is_some_and(|m| m.text.contains("fn main()"))).await?;
    let streaming = this.read_with(cx, |v, _| last_assistant(v).map(|m| m.text).unwrap_or_default()).map_err(|e| e.to_string())?;
    if !bragi_core::markdown::parse_streaming(&streaming).blocks.iter().any(|b| matches!(b, bragi_core::markdown::Block::Code { closed: false, .. }))
    {
        return Err(format!("a half-open fence did not render as code: {streaming:?}"));
    }
    shot(cx, "25-markdown-streaming").await;
    wait_for(&this, cx, "the answer", |v| v.streaming.is_none() && last_assistant(v).is_some_and(|m| m.state == MessageState::Complete)).await?;
    let text = this.read_with(cx, |v, _| last_assistant(v).map(|m| m.text).unwrap_or_default()).map_err(|e| e.to_string())?;
    let doc = bragi_core::markdown::parse(&text);
    use bragi_core::markdown::Block as B;
    let has = |f: fn(&B) -> bool| doc.blocks.iter().any(f);
    if !(has(|b| matches!(b, B::Heading { .. }))
        && has(|b| matches!(b, B::List { .. }))
        && has(|b| matches!(b, B::Table { .. }))
        && has(|b| matches!(b, B::Code { closed: true, .. }))
        && has(|b| matches!(b, B::Paragraph { spans } if spans.iter().any(|s| s.bold && s.text == "4"))))
    {
        return Err(format!("the Markdown reply did not parse into every block: {doc:?}"));
    }
    let key = this
        .read_with(cx, |v, _| v.current.as_ref().map(|c| format!("{}-{}", c.id, c.messages.len() - 1)))
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let code = doc.code_blocks()[0].to_string();
    let got = this
        .update(cx, |v, cx| {
            v.copy_text(SharedString::from(format!("msg-{key}")), text.clone(), cx);
            let whole = cx.read_from_clipboard().and_then(|i| i.text());
            v.copy_text(SharedString::from(format!("{key}-code")), code.clone(), cx);
            (whole, cx.read_from_clipboard().and_then(|i| i.text()))
        })
        .map_err(|e| e.to_string())?;
    if got.0.as_deref() != Some(text.as_str()) || got.1.as_deref() != Some(code.as_str()) {
        return Err("Copy did not give the raw Markdown and the code".into());
    }
    step("markdown");
    shot(cx, "26-markdown").await;
    // The model's reasoning streams in an open Thinking section, which closes when the answer starts.
    let prompt = format!("{} which node answers?", bragi_core::demo::THINK_PROMPT);
    this.update(cx, |v, cx| {
        if v.agent {
            v.toggle_agent(cx);
        }
        v.new_chat(cx);
        v.pick_model(bragi_core::demo::WARM_MODEL, cx);
        v.set_prompt(&prompt, cx)
    })
    .map_err(|e| e.to_string())?;
    press_send(&this, cx)?;
    wait_for(&this, cx, "the reasoning", |v| last_assistant(v).is_some_and(|m| m.reasoning.split(' ').count() > 5 && m.text.is_empty())).await?;
    step("thinking");
    shot(cx, "27-thinking").await;
    wait_for(&this, cx, "the answer", |v| v.streaming.is_none() && last_assistant(v).is_some_and(|m| m.state == MessageState::Complete)).await?;
    let (text, reasoning, ms) =
        this.read_with(cx, |v, _| last_assistant(v).map(|m| (m.text, m.reasoning, m.thinking_ms)).unwrap_or_default()).map_err(|e| e.to_string())?;
    let want = bragi_core::demo::reasoning_for(&prompt).unwrap_or_default();
    if text != bragi_core::demo::answer_for(bragi_core::demo::WARM_MODEL, &prompt) || reasoning.trim() != want || ms.is_none() {
        return Err(format!("thinking: {text:?} {reasoning:?} {ms:?}"));
    }
    step("thought");
    shot(cx, "28-thought").await;
    // A click opens the closed section again.
    this.update(cx, |v, cx| {
        if let Some(c) = &v.current {
            v.thinking_flipped.insert((c.id.clone(), c.messages.len() - 1));
        }
        cx.notify();
    })
    .map_err(|e| e.to_string())?;
    shot(cx, "29-thought-opened").await;
    Ok(())
}

/// Presses Send (it needs the window).
fn press_send(this: &WeakEntity<BragiView>, cx: &mut gpui::AsyncApp) -> Result<(), String> {
    let handle = cx.update(|cx| cx.windows().first().copied()).ok_or("no window")?;
    let this = this.clone();
    handle
        .update(cx, |_, window, cx| {
            let _ = this.update(cx, |v, cx| v.send(window, cx));
        })
        .map_err(|e| e.to_string())
}

// ---- start-up --------------------------------------------------------------------------------

fn default_home() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("bragi")
}

fn label() -> String {
    let host = std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).unwrap_or_default();
    if host.is_empty() { "Bragi desktop".into() } else { format!("Bragi on {host}") }
}

pub fn run(launch: Launch) -> std::process::ExitCode {
    // The demo: an in-process inference node with a test identity, and a throwaway profile
    // unless --home names one.
    let mut _scratch = None;
    let mut _runtime = None;
    let mut demo: Option<Arc<DemoNode>> = None;
    let core = if launch.demo {
        let home = match launch.home.clone() {
            Some(h) => h,
            None => {
                let d = tempfile::tempdir().expect("a temp dir");
                let p = d.path().to_path_buf();
                _scratch = Some(d);
                p
            }
        };
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("a runtime");
        // Two inference nodes of one user, so the pool has something to merge and fail over.
        let node = rt.block_on(DemoNode::start_cloud(&home.join("demo"), Duration::from_millis(45))).expect("the demo nodes");
        let _ = FAULTS.set(node.faults.clone());
        let _ = DEMO.set(node.clone());
        let core = {
            let _g = rt.enter();
            Core::start(node.app_config(&home.join("app"), "Bragi desktop (demo)")).expect("bragi-core")
        };
        node.auto_accept_after(&core, Duration::from_millis(1500));
        demo = Some(node);
        _runtime = Some(rt);
        core
    } else {
        let home = launch.home.clone().unwrap_or_else(default_home);
        let mut cfg = Config::new(home, label());
        cfg.transport = Transport::Internet;
        match Core::start(cfg) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("bragi: {e}");
                return std::process::ExitCode::FAILURE;
            }
        }
    };
    let play = launch.scenario;
    let app = gpui_platform::application();
    let for_app = core.clone();
    app.run(move |cx: &mut App| {
        let mut keys = text_input::bindings();
        keys.push(KeyBinding::new("escape", Stop, Some("Bragi")));
        // The text field's own Home/End win while it has focus; these reach the list otherwise.
        keys.push(KeyBinding::new("pageup", ChatPageUp, Some("Bragi")));
        keys.push(KeyBinding::new("pagedown", ChatPageDown, Some("Bragi")));
        keys.push(KeyBinding::new("home", ChatTop, Some("Bragi")));
        keys.push(KeyBinding::new("end", ChatBottom, Some("Bragi")));
        keys.push(KeyBinding::new("ctrl-home", ChatTop, Some("Bragi")));
        keys.push(KeyBinding::new("ctrl-end", ChatBottom, Some("Bragi")));
        keys.push(KeyBinding::new("ctrl-q", Quit, None));
        cx.bind_keys(keys);
        let bounds = Bounds::centered(None, size(px(1100.), px(720.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            app_id: Some("bragi".into()),
            window_min_size: Some(size(px(560.), px(400.))),
            window_decorations: Some(if cfg!(target_os = "linux") { WindowDecorations::Client } else { WindowDecorations::Server }),
            titlebar: (!cfg!(target_os = "linux")).then(|| gpui::TitlebarOptions {
                title: None,
                appears_transparent: true,
                traffic_light_position: Some(gpui::point(px(12.), px(12.))),
            }),
            ..Default::default()
        };
        let core2 = for_app.clone();
        let opened = cx.open_window(options, move |window, cx| cx.new(|cx| BragiView::new(core2, window, cx)));
        let Ok(handle) = opened else {
            eprintln!("bragi: cannot open a window");
            EXIT.store(1, Ordering::SeqCst);
            cx.quit();
            return;
        };
        let _ = handle.update(cx, |_, window, _| window.set_window_title("Bragi"));
        if play {
            let view = handle.entity(cx).expect("the view").downgrade();
            cx.spawn(async move |cx| {
                let result = scenario(view, cx).await;
                match result {
                    Ok(()) => println!("bragi-scenario: passed"),
                    Err(e) => {
                        println!("bragi-scenario: FAILED: {e}");
                        EXIT.store(1, Ordering::SeqCst);
                    }
                }
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
        cx.activate(true);
    });
    core.shutdown();
    drop(demo);
    std::process::ExitCode::from(EXIT.load(Ordering::SeqCst))
}

#[cfg(test)]
mod tests {
    use super::splice;

    #[test]
    fn a_delta_lands_at_its_offset() {
        let mut t = "héllo world".to_string();
        splice(&mut t, 6, "there");
        assert_eq!(t, "héllo there");
        splice(&mut t, 20, "!");
        assert_eq!(t, "héllo there!");
    }
}
