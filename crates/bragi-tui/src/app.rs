//! The terminal app's state and what each key does. Drawing is `ui`; behaviour is
//! `bragi-core`'s: this only keeps what the screen shows and calls `Core`.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use bragi_core::{AppState, Attachment, ChatEvent, ChatHandle, Conversation, ConversationSummary, Core, ErrorKind, LinkCheck, PairState};
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::sync::{mpsc as tmpsc, watch};

use crate::code::{CodeState, Command, Phase};
use crate::editor::Editor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Chat,
    Link,
    Code,
    Status,
    Models,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Input,
    Chats,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Chat,
    Code,
}

/// Results of the core's async actions, back on the UI thread.
enum Job {
    Offer(Result<String, String>),
    Accept(Result<LinkCheck, String>),
    AcceptDone(bool, Result<(), String>),
}

/// An answer streaming in.
pub struct Stream {
    pub request_id: String,
    pub conversation_id: String,
    pub text: String,
    pub reasoning: String,
    pub node: Option<String>,
    pub model: Option<String>,
    pub queued: Option<u32>,
    pub purpose: Purpose,
    events: tmpsc::UnboundedReceiver<ChatEvent>,
}

/// How chats are sent. Today the user's Weirdnet pool; OpenAI-compatible servers join this list
/// with the endpoints feature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Weirdnet,
}

impl Target {
    pub fn label(&self) -> &'static str {
        match self {
            Target::Weirdnet => "Weirdnet",
        }
    }
}

pub struct App {
    pub core: Core,
    pub state: AppState,
    watch: watch::Receiver<AppState>,
    pub view: View,
    back: View,
    /// The link view was opened because the device wasn't linked (not by the user).
    auto_link: bool,
    pub focus: Focus,
    pub input: Editor,
    pub attachments: Vec<Attachment>,
    pub chats: Vec<ConversationSummary>,
    pub chat_sel: usize,
    pub current: Option<Conversation>,
    pub stream: Option<Stream>,
    pub target: Target,
    pub model: Option<String>,
    pub model_sel: usize,
    pub notice: Option<String>,
    /// Lines scrolled up from the bottom of the transcript (or the code answer).
    pub scroll: usize,
    pub link_input: Editor,
    pub link_check: Option<LinkCheck>,
    pub code: CodeState,
    pub confirm_delete: Option<String>,
    /// Text to put on the terminal's clipboard (OSC 52), written by the main loop.
    pub clipboard: Vec<String>,
    pub quit: bool,
    jobs: (mpsc::Sender<Job>, mpsc::Receiver<Job>),
    last_reload: Instant,
}

fn linked(s: &AppState) -> bool {
    matches!(s.pair, PairState::Linked { .. })
}

impl App {
    /// `home` is the profile folder (the code undo journal lives in it); `root` is the folder
    /// code mode works in.
    pub fn new(core: Core, home: &Path, root: PathBuf) -> Self {
        let watch = core.watch();
        let state = core.state();
        let mut app = Self {
            state: state.clone(),
            watch,
            view: View::Chat,
            back: View::Chat,
            auto_link: false,
            focus: Focus::Input,
            input: Editor::default(),
            attachments: Vec::new(),
            chats: Vec::new(),
            chat_sel: 0,
            current: None,
            stream: None,
            target: Target::Weirdnet,
            model: None,
            model_sel: 0,
            notice: None,
            scroll: 0,
            link_input: Editor::default(),
            link_check: None,
            code: CodeState::new(root, home.join("code-undo")),
            confirm_delete: None,
            clipboard: Vec::new(),
            quit: false,
            jobs: mpsc::channel(),
            last_reload: Instant::now(),
            core,
        };
        // A linked device goes online in the background: give it a moment before showing the
        // link screen.
        if !linked(&state) && !app.core.has_identity() {
            app.view = View::Link;
            app.auto_link = true;
        }
        app.reload();
        app
    }

    pub fn linked(&self) -> bool {
        linked(&self.state)
    }

    pub fn streaming(&self) -> bool {
        self.stream.is_some()
    }

    /// The model chats go to: the picked one, else the catalog's default.
    pub fn model_label(&self) -> String {
        match (&self.model, self.state.catalog.default_model()) {
            (Some(m), _) => m.clone(),
            (None, Some(d)) => format!("auto ({d})"),
            (None, None) => "auto".into(),
        }
    }

    fn reload(&mut self) {
        self.chats = self.core.conversations(0, 200).unwrap_or_default();
        self.chat_sel = self.chat_sel.min(self.chats.len().saturating_sub(1));
        if let Some(c) = &self.current {
            if let Ok(c) = self.core.conversation(&c.id) {
                self.current = Some(c);
            }
        }
        self.last_reload = Instant::now();
    }

    fn say(&mut self, s: impl Into<String>) {
        self.notice = Some(s.into());
    }

    // ---- the loop's side: state, jobs, streams ------------------------------------------

    /// Takes in whatever changed since the last frame. Call before every draw.
    pub fn tick(&mut self) {
        if self.watch.has_changed().unwrap_or(false) {
            let was = self.linked();
            self.state = self.watch.borrow_and_update().clone();
            if !was && self.linked() {
                self.reload();
                if self.view == View::Link && self.auto_link {
                    self.view = View::Chat;
                }
                self.auto_link = false;
                self.say("Linked. Type a message and press Enter.");
            }
        }
        while let Ok(job) = self.jobs.1.try_recv() {
            self.job(job);
        }
        self.drain_stream();
        if self.stream.is_none() && self.last_reload.elapsed() > Duration::from_secs(3) {
            // Conversations from the user's other devices arrive by sync.
            self.reload();
        }
    }

    fn job(&mut self, job: Job) {
        match job {
            Job::Offer(Ok(_)) => self.say("Open this code on a device that's already linked; c copies it."),
            Job::Offer(Err(e)) | Job::Accept(Err(e)) => self.say(e),
            Job::Accept(Ok(check)) => {
                self.say(format!("Does {} show {}? y yes · n no", check.device, check.sas));
                self.link_check = Some(check);
            }
            Job::AcceptDone(matches, r) => {
                self.say(match r {
                    Ok(()) if matches => "Linked.".to_string(),
                    Ok(()) => "Cancelled.".to_string(),
                    Err(e) => e,
                });
            }
        }
    }

    fn drain_stream(&mut self) {
        let mut finished: Option<Result<(), (ErrorKind, String)>> = None;
        if let Some(s) = &mut self.stream {
            while let Ok(ev) = s.events.try_recv() {
                match ev {
                    ChatEvent::Queued { position } => s.queued = Some(position),
                    ChatEvent::Started { node, model, .. } => {
                        s.queued = None;
                        s.node = Some(node);
                        s.model = Some(model);
                    }
                    ChatEvent::Delta { offset, text } => {
                        truncate_chars(&mut s.text, offset as usize);
                        s.text.push_str(&text);
                    }
                    ChatEvent::Reasoning { offset, text } => {
                        truncate_chars(&mut s.reasoning, offset as usize);
                        s.reasoning.push_str(&text);
                    }
                    ChatEvent::Resumed { node, offset } => {
                        s.node = Some(node);
                        truncate_chars(&mut s.text, offset as usize);
                    }
                    ChatEvent::Done { .. } => finished = Some(Ok(())),
                    ChatEvent::Error { kind, message, .. } => finished = Some(Err((kind, message))),
                    ChatEvent::Notice { text } => self.notice = Some(text),
                    ChatEvent::Retry { attempt, .. } => self.notice = Some(format!("Trying again (attempt {attempt})…")),
                    _ => {}
                }
                if finished.is_some() {
                    break;
                }
            }
        }
        let Some(result) = finished else { return };
        let Some(s) = self.stream.take() else { return };
        match (s.purpose, result) {
            (Purpose::Chat, r) => {
                self.reload();
                self.notice = match r {
                    Ok(()) => None,
                    Err((ErrorKind::Cancelled, _)) => Some("Stopped. Ctrl-R continues it.".into()),
                    Err((_, m)) => Some(m),
                };
            }
            (Purpose::Code, Ok(())) => {
                let note = self.code.answered(&s.text);
                self.scroll = 0;
                self.say(note);
            }
            (Purpose::Code, Err((kind, m))) => {
                self.code.failed();
                self.say(if kind == ErrorKind::Cancelled { "Stopped.".to_string() } else { m });
            }
        }
    }

    fn follow(&mut self, h: ChatHandle, purpose: Purpose) {
        self.scroll = 0;
        if purpose == Purpose::Chat {
            self.current = self.core.conversation(&h.conversation_id).ok();
            self.reload();
            if let Some(i) = self.chats.iter().position(|c| c.id == h.conversation_id) {
                self.chat_sel = i;
            }
        }
        self.stream = Some(Stream {
            request_id: h.request_id,
            conversation_id: h.conversation_id,
            text: String::new(),
            reasoning: String::new(),
            node: None,
            model: None,
            queued: None,
            purpose,
            events: h.events,
        });
    }

    // ---- input --------------------------------------------------------------------------

    pub fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Key(k) if k.kind != KeyEventKind::Release => self.on_key(k),
            Event::Paste(text) => match self.view {
                View::Link => self.link_input.insert(text.trim()),
                View::Chat | View::Code if !(self.view == View::Code && self.code.phase == Phase::Review) => {
                    self.focus = Focus::Input;
                    self.input.insert(&text)
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn open(&mut self, view: View) {
        if view == self.view {
            return;
        }
        if matches!(view, View::Help | View::Models | View::Status) && !matches!(self.view, View::Help | View::Models | View::Status) {
            self.back = self.view;
        }
        if view == View::Models {
            let current = self.model.clone().or_else(|| self.state.catalog.default_model().map(str::to_string));
            self.model_sel = self.state.catalog.models.iter().position(|m| Some(&m.id) == current.as_ref()).unwrap_or(0);
        }
        self.auto_link = false;
        self.view = view;
    }

    fn close(&mut self) {
        self.view = if matches!(self.view, View::Help | View::Models | View::Status) { self.back } else { View::Chat };
    }

    pub fn on_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        // Global keys first.
        match k.code {
            KeyCode::Char('q') if ctrl => return self.quit = true,
            KeyCode::Char('c') if ctrl => {
                if let Some(s) = &self.stream {
                    self.core.cancel(&s.request_id);
                } else {
                    self.quit = true;
                }
                return;
            }
            KeyCode::F(1) => return if self.view == View::Help { self.close() } else { self.open(View::Help) },
            KeyCode::Char('l') if ctrl => return self.open(View::Link),
            KeyCode::F(2) => return self.open(View::Link),
            KeyCode::Char('o') if ctrl => return self.open(View::Models),
            KeyCode::F(3) => return self.open(View::Models),
            KeyCode::Char('e') if ctrl && self.view != View::Code && self.input.is_empty() => return self.open(View::Code),
            KeyCode::F(4) => return self.open(View::Code),
            KeyCode::Char('t') if ctrl => return self.open(View::Status),
            KeyCode::F(5) => return self.open(View::Status),
            KeyCode::Char('n') if ctrl => {
                self.open(View::Chat);
                self.current = None;
                self.focus = Focus::Input;
                self.scroll = 0;
                return;
            }
            KeyCode::Char('y') if ctrl => return self.copy(),
            _ => {}
        }
        if let Some(id) = self.confirm_delete.take() {
            if k.code == KeyCode::Char('y') {
                match self.core.delete_conversation(&id) {
                    Ok(()) => {
                        if self.current.as_ref().is_some_and(|c| c.id == id) {
                            self.current = None;
                        }
                        self.reload();
                        self.say("Deleted.");
                    }
                    Err(e) => self.say(e.message),
                }
            } else {
                self.say("Kept.");
            }
            return;
        }
        match self.view {
            View::Help => {
                if matches!(k.code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?')) {
                    self.close();
                }
            }
            View::Models => self.models_key(k),
            View::Status => match k.code {
                KeyCode::Esc | KeyCode::Char('q') => self.close(),
                KeyCode::Char('r') => {
                    let core = self.core.clone();
                    self.core.handle().spawn(async move {
                        let _ = core.sync_now().await;
                    });
                    self.say("Syncing with your other devices…");
                }
                _ => {}
            },
            View::Link => self.link_key(k, ctrl),
            View::Code => self.code_key(k, ctrl, alt),
            View::Chat => self.chat_key(k, ctrl, alt),
        }
    }

    fn copy(&mut self) {
        let text = match (&self.view, &self.state.pair) {
            (View::Link, PairState::Offering { link, .. }) => Some(link.clone()),
            (View::Code, _) if !self.code.diffs.is_empty() => Some(bragi_core::code::diff::unified_all(&self.code.diffs)),
            (View::Code, _) if !self.code.answer.is_empty() => Some(self.code.answer.clone()),
            _ => self.current.as_ref().and_then(|c| c.messages.iter().rev().find(|m| m.role == bragi_core::Role::Assistant)).map(|m| m.text.clone()),
        };
        match text {
            Some(t) if !t.is_empty() => {
                self.clipboard.push(t);
                self.say("Copied to the terminal's clipboard (OSC 52).");
            }
            _ => self.say("Nothing to copy here."),
        }
    }

    fn models_key(&mut self, k: KeyEvent) {
        let n = self.state.catalog.models.len();
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => self.close(),
            KeyCode::Up | KeyCode::Char('k') => self.model_sel = self.model_sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.model_sel = (self.model_sel + 1).min(n.saturating_sub(1)),
            KeyCode::Char('a') | KeyCode::Backspace => {
                self.model = None;
                self.say("Model: automatic (the first warm one).");
                self.close();
            }
            KeyCode::Enter => {
                if let Some(m) = self.state.catalog.models.get(self.model_sel) {
                    self.model = Some(m.id.clone());
                    self.say(format!("Model: {}", m.id));
                }
                self.close();
            }
            _ => {}
        }
    }

    fn link_key(&mut self, k: KeyEvent, ctrl: bool) {
        if let Some(check) = self.link_check.clone() {
            match k.code {
                KeyCode::Char('y') | KeyCode::Char('n') | KeyCode::Esc => {
                    let matches = k.code == KeyCode::Char('y');
                    self.link_check = None;
                    let (core, tx) = (self.core.clone(), self.jobs.0.clone());
                    self.core.handle().spawn(async move {
                        let r = core.pair_accept_confirm(matches).await.map_err(|e| e.message);
                        let _ = tx.send(Job::AcceptDone(matches, r));
                    });
                    self.say(if matches { format!("Linking with {}…", check.device) } else { "Cancelling…".into() });
                }
                _ => {}
            }
            return;
        }
        let empty = self.link_input.is_empty();
        match (&self.state.pair, k.code) {
            (PairState::Confirm { .. }, KeyCode::Char(c @ ('y' | 'n'))) if empty => {
                if let Err(e) = self.core.pair_confirm(c == 'y') {
                    self.say(e.message);
                }
            }
            (PairState::Offering { .. }, KeyCode::Char('c')) if empty => self.copy(),
            (PairState::Offering { .. } | PairState::Confirm { .. } | PairState::Waiting { .. }, KeyCode::Esc) if empty => {
                self.core.pair_cancel();
                self.say("Stopped linking.");
            }
            (PairState::Unpaired | PairState::Failed { .. }, KeyCode::Char('s')) if empty => {
                let (core, tx) = (self.core.clone(), self.jobs.0.clone());
                self.core.handle().spawn(async move {
                    let r = core.pair_offer().await.map_err(|e| e.message);
                    let _ = tx.send(Job::Offer(r));
                });
                self.say("Making a link code…");
            }
            (_, KeyCode::Esc) => {
                if empty {
                    self.close();
                } else {
                    self.link_input.take();
                }
            }
            (_, KeyCode::Enter) => self.submit_link(),
            _ => {
                if !ctrl || matches!(k.code, KeyCode::Char('a' | 'e' | 'u' | 'w')) {
                    self.link_input.key(k);
                }
            }
        }
    }

    /// The link box: a node address (`/ip4/…/p2p/…`, from `ymirctl pool addresses`) is dialed from
    /// now on; anything else is a link code shown by another device.
    pub fn submit_link(&mut self) {
        let text = self.link_input.text().trim().to_string();
        if text.is_empty() {
            return;
        }
        if text.starts_with('/') {
            match self.core.add_peer(&text) {
                Ok(()) => {
                    self.link_input.take();
                    self.say("Added the node's address; looking for it…");
                }
                Err(e) => self.say(e.message),
            }
            return;
        }
        self.link_input.take();
        let (core, tx) = (self.core.clone(), self.jobs.0.clone());
        self.core.handle().spawn(async move {
            let r = core.pair_accept(&text).await.map_err(|e| e.message);
            let _ = tx.send(Job::Accept(r));
        });
        self.say("Opening the code…");
    }

    fn scroll_key(&mut self, k: KeyCode) -> bool {
        match k {
            KeyCode::PageUp => self.scroll += 10,
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(10),
            _ => return false,
        }
        true
    }

    fn chat_key(&mut self, k: KeyEvent, ctrl: bool, alt: bool) {
        if self.scroll_key(k.code) {
            return;
        }
        if k.code == KeyCode::Tab || k.code == KeyCode::BackTab {
            self.focus = if self.focus == Focus::Input { Focus::Chats } else { Focus::Input };
            return;
        }
        if k.code == KeyCode::Char('r') && ctrl {
            return self.continue_answer();
        }
        if k.code == KeyCode::Esc {
            if let Some(s) = &self.stream {
                self.core.cancel(&s.request_id);
            } else if self.focus == Focus::Chats {
                self.focus = Focus::Input;
            }
            return;
        }
        if self.focus == Focus::Chats {
            match k.code {
                KeyCode::Up | KeyCode::Char('k') => self.chat_sel = self.chat_sel.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => self.chat_sel = (self.chat_sel + 1).min(self.chats.len().saturating_sub(1)),
                KeyCode::Enter => {
                    if let Some(c) = self.chats.get(self.chat_sel) {
                        self.current = self.core.conversation(&c.id).ok();
                        self.scroll = 0;
                        self.focus = Focus::Input;
                    }
                }
                KeyCode::Delete | KeyCode::Char('d') => {
                    if let Some(c) = self.chats.get(self.chat_sel) {
                        self.confirm_delete = Some(c.id.clone());
                        self.say(format!("Delete \"{}\"? y yes · any other key keeps it", c.title));
                    }
                }
                KeyCode::Char('?') => self.open(View::Help),
                _ => {}
            }
            return;
        }
        match k.code {
            KeyCode::Enter if alt => self.input.insert("\n"),
            KeyCode::Char('j') if ctrl => self.input.insert("\n"),
            KeyCode::Enter => self.send(),
            KeyCode::Char('?') if self.input.is_empty() => self.open(View::Help),
            KeyCode::Up if self.input.is_empty() => self.scroll += 1,
            KeyCode::Down if self.input.is_empty() => self.scroll = self.scroll.saturating_sub(1),
            _ => {
                self.input.key(k);
            }
        }
    }

    /// Sends the input: `/attach PATH` adds a text file to the next message, `/detach` drops them.
    pub fn send(&mut self) {
        if self.stream.is_some() {
            return self.say("An answer is still coming; Esc stops it.");
        }
        let text = self.input.text().trim().to_string();
        if text.is_empty() {
            return;
        }
        if let Some(path) = text.strip_prefix("/attach ") {
            match Attachment::from_path(&crate::code::expand(path.trim())) {
                Ok(a) => {
                    self.say(format!("Attached {} to the next message.", a.name));
                    self.attachments.push(a);
                    self.input.take();
                }
                Err(e) => self.say(e.message),
            }
            return;
        }
        if text == "/detach" {
            self.attachments.clear();
            self.input.take();
            return self.say("No attachments.");
        }
        let conv = self.current.as_ref().map(|c| c.id.clone());
        let files = std::mem::take(&mut self.attachments);
        match self.core.send_with(conv.as_deref(), &text, self.model.clone(), files.clone()) {
            Ok(h) => {
                self.input.take();
                self.notice = None;
                self.follow(h, Purpose::Chat);
            }
            Err(e) => {
                self.attachments = files;
                self.say(e.message);
            }
        }
    }

    pub fn continue_answer(&mut self) {
        let Some(c) = &self.current else { return };
        if self.stream.is_some() || !c.continuable() {
            return self.say("Nothing to continue.");
        }
        match self.core.continue_answer(&c.id, self.model.clone()) {
            Ok(h) => self.follow(h, Purpose::Chat),
            Err(e) => self.say(e.message),
        }
    }

    fn code_key(&mut self, k: KeyEvent, ctrl: bool, alt: bool) {
        if self.scroll_key(k.code) {
            return;
        }
        match self.code.phase {
            Phase::Review => match k.code {
                KeyCode::Up | KeyCode::Char('k') => self.code.step(false),
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => self.code.step(true),
                KeyCode::Char('y') => {
                    self.code.set_current(true);
                    self.code.step(true);
                }
                KeyCode::Char('n') | KeyCode::Char('x') => {
                    self.code.set_current(false);
                    self.code.step(true);
                }
                KeyCode::Char(' ') => self.code.toggle_current(),
                KeyCode::Char('a') => self.code.set_all(true),
                KeyCode::Char('r') => self.code.set_all(false),
                KeyCode::Char('v') => self.code.side_by_side = !self.code.side_by_side,
                KeyCode::Char('w') | KeyCode::Enter => match self.code.write() {
                    Ok(n) => self.say(n),
                    Err(e) => self.say(e),
                },
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.code.discard();
                    self.say("Discarded the proposed changes.");
                }
                KeyCode::Char('?') => self.open(View::Help),
                _ => {}
            },
            Phase::Asking => {
                if k.code == KeyCode::Esc
                    && let Some(s) = &self.stream
                {
                    self.core.cancel(&s.request_id);
                }
            }
            Phase::Compose => match k.code {
                KeyCode::Esc if self.input.is_empty() => self.view = View::Chat,
                KeyCode::Esc => {
                    self.input.take();
                }
                KeyCode::Enter if alt => self.input.insert("\n"),
                KeyCode::Char('j') if ctrl => self.input.insert("\n"),
                KeyCode::Enter => self.code_submit(),
                KeyCode::Char('?') if self.input.is_empty() => self.open(View::Help),
                _ => {
                    self.input.key(k);
                }
            },
        }
    }

    /// Runs the code-mode line in the input box.
    pub fn code_submit(&mut self) {
        let line = self.input.text().to_string();
        let cmd = match self.code.parse(&line) {
            Ok(c) => c,
            Err(e) => return self.say(e),
        };
        self.input.take();
        let note = match cmd {
            Command::Nothing => return,
            Command::Add(paths) => self.code.add(&paths),
            Command::Drop(what) => self.code.drop_path(&what),
            Command::Clear => {
                self.code.context.clear();
                self.code.refresh().map(|_| "Context cleared.".to_string())
            }
            Command::Root(dir) => {
                let dir = crate::code::expand(&dir);
                let dir = if dir.is_absolute() { dir } else { self.code.root.join(dir) };
                if dir.is_dir() {
                    self.code.root = dir;
                    self.code.context.clear();
                    self.code.refresh().map(|_| format!("Working in {}", self.code.root.display()))
                } else {
                    Err(format!("{} isn't a folder", dir.display()))
                }
            }
            Command::Undo { force } => self.code.undo(force),
            Command::Ask(req) => {
                if self.stream.is_some() {
                    return self.say("An answer is still coming; Esc stops it.");
                }
                let title = bragi_core::code::prompt::title(&req);
                match CodeState::messages(&req).and_then(|m| self.core.complete(m, self.model.clone()).map_err(|e| e.message)) {
                    Ok(h) => {
                        self.code.start(req);
                        self.follow(h, Purpose::Code);
                        Ok(format!("{title} — asking {}…", self.model_label()))
                    }
                    Err(e) => Err(e),
                }
            }
        };
        match note {
            Ok(n) => self.say(n),
            Err(e) => self.say(e),
        }
    }
}

fn truncate_chars(s: &mut String, chars: usize) {
    if let Some((i, _)) = s.char_indices().nth(chars) {
        s.truncate(i);
    }
}
