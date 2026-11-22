//! The terminal app's state and what each key does. Drawing is `ui`; behaviour is
//! `bragi-core`'s: this only keeps what the screen shows and calls `Core`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use bragi_core::{
    AppState, Attachment, ChatEvent, ChatHandle, Conversation, Core, ErrorKind, Folder, KeyChange, LinkCheck, PairState, SessionSummary, Target,
    TargetOption, TestReport,
};
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::sync::{mpsc as tmpsc, watch};

use crate::code::{CodeState, Command, Phase};
use crate::editor::Editor;
use crate::settings::{self, Form, Settings};
use crate::tree::{self, Row};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Chat,
    Link,
    Code,
    Status,
    Models,
    Settings,
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
    Test(String, Result<TestReport, String>),
    Models(String, Result<Vec<String>, String>),
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
    /// When the first reasoning arrived, and how long it took once the answer began.
    pub thinking_since: Option<Instant>,
    pub thought_ms: Option<u64>,
    /// When it was sent (the spinner's clock).
    pub started: Instant,
    pub purpose: Purpose,
    events: tmpsc::UnboundedReceiver<ChatEvent>,
}

pub struct App {
    pub core: Core,
    pub state: AppState,
    watch: watch::Receiver<AppState>,
    pub view: View,
    pub back: View,
    /// The link view was opened because the device wasn't linked (not by the user).
    auto_link: bool,
    pub focus: Focus,
    pub input: Editor,
    pub attachments: Vec<Attachment>,
    pub sessions: Vec<SessionSummary>,
    pub folders: Vec<Folder>,
    /// The sidebar's rows (folders and chats) and the folders folded shut.
    pub rows: Vec<Row>,
    pub closed: HashSet<String>,
    pub chat_sel: usize,
    pub current: Option<Conversation>,
    pub stream: Option<Stream>,
    /// Where the next message goes: `None` uses the chat's own target (a new chat the default).
    pub target: Option<Target>,
    pub targets: Vec<TargetOption>,
    pub model: Option<String>,
    pub model_sel: usize,
    /// The picker: the target under the cursor (its models are listed), and whether the
    /// cursor is in the models list rather than the targets.
    pub picker_target: usize,
    pub picker_models_pane: bool,
    /// The models a picked endpoint serves (listed in the picker), and which endpoint.
    pub endpoint_models: Vec<String>,
    pub models_for: Option<String>,
    pub settings: Settings,
    /// The folder a `/folder` typed after `n` in the sidebar goes in.
    pub new_folder_parent: Option<String>,
    /// Show the reasoning of every answer, not only the title line.
    pub show_thinking: bool,
    pub notice: Option<String>,
    /// How the notice reads (its toast's colour), and when it first showed.
    pub notice_tone: Tone,
    pub notice_since: Option<(String, Instant)>,
    /// Lines scrolled down in the help.
    pub help_scroll: usize,
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

/// A notice's kind: the colour and icon of its toast, and whether it waits for an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Ok,
    /// A question (y/n): it stays until answered.
    Ask,
    Error,
}

/// How long a toast stays up.
pub const TOAST_MS: u128 = 5000;

fn tone_of(text: &str) -> Tone {
    const OK: &[&str] =
        &["Saved", "Deleted", "Linked", "Wrote", "Undid", "Copied", "Added", "Attached", "Moved", "Renamed", "Made", "OK in", "Model:"];
    if text.contains("? y yes") || text.starts_with("Does ") {
        Tone::Ask
    } else if OK.iter().any(|w| text.starts_with(w)) {
        Tone::Ok
    } else {
        Tone::Info
    }
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
            sessions: Vec::new(),
            folders: Vec::new(),
            rows: Vec::new(),
            closed: HashSet::new(),
            chat_sel: 0,
            current: None,
            stream: None,
            target: None,
            targets: Vec::new(),
            model: None,
            model_sel: 0,
            picker_target: 0,
            picker_models_pane: false,
            endpoint_models: Vec::new(),
            models_for: None,
            settings: Settings::default(),
            show_thinking: false,
            new_folder_parent: None,
            notice: None,
            notice_tone: Tone::Info,
            notice_since: None,
            scroll: 0,
            help_scroll: 0,
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

    /// Starts in code mode on what `bragi-tui PATH` named: the folder's files, or the one file,
    /// are the context. Requests and diffs stay inside the root.
    pub fn open_code(&mut self, o: &crate::cli::Opened) {
        match self.code.add(&[o.context.display().to_string()]) {
            Ok(note) => self.say(&note),
            Err(e) => self.fail(e),
        }
        // A new device links first; the code view waits behind that screen.
        if self.view == View::Link {
            self.back = View::Code;
        } else {
            self.view = View::Code;
        }
    }

    pub fn linked(&self) -> bool {
        linked(&self.state)
    }

    pub fn streaming(&self) -> bool {
        self.stream.is_some()
    }

    /// Where the next message goes: the picked target, else the open chat's, else the default.
    pub fn effective_target(&self) -> Option<Target> {
        self.target.clone().or_else(|| self.current.as_ref().and_then(|c| self.core.session_target(&c.id).target))
    }

    /// The words for [`effective_target`](Self::effective_target).
    pub fn target_label(&self) -> String {
        match self.effective_target() {
            None => self.targets.first().map(|t| t.label.clone()).unwrap_or_else(|| "Weirdnet pool".into()),
            Some(t) => self.targets.iter().find(|o| o.target == t).map(|o| o.label.clone()).unwrap_or_else(|| match &t {
                Target::Pool => "Weirdnet pool".into(),
                Target::Endpoint { id, model } => {
                    let name = self.settings.endpoints.iter().find(|e| &e.id == id).map(|e| e.name.clone()).unwrap_or_else(|| "endpoint".into());
                    if model.is_empty() { name } else { format!("{name} · {model}") }
                }
            }),
        }
    }

    /// The model chats go to: the picked one, else the catalog's default.
    pub fn model_label(&self) -> String {
        if let Some(Target::Endpoint { model, .. }) = self.effective_target() {
            return if model.is_empty() { "endpoint default".into() } else { model };
        }
        match (&self.model, self.state.catalog.default_model()) {
            (Some(m), _) => m.clone(),
            (None, Some(d)) => format!("auto ({d})"),
            (None, None) => "auto".into(),
        }
    }

    fn reload(&mut self) {
        self.sessions = self.core.sessions(0, 500).unwrap_or_default();
        self.folders = self.core.folders();
        self.rows = tree::rows(&self.folders, &self.sessions, &self.closed);
        self.chat_sel = self.chat_sel.min(self.rows.len().saturating_sub(1));
        self.targets = self.core.targets();
        self.settings.reload(&self.core);
        if let Some(c) = &self.current
            && let Ok(c) = self.core.conversation(&c.id)
        {
            self.current = Some(c);
        }
        self.last_reload = Instant::now();
    }

    fn say(&mut self, s: impl Into<String>) {
        let s = s.into();
        self.notice_tone = tone_of(&s);
        self.notice = Some(s);
        self.notice_since = None;
    }

    /// A notice that something went wrong.
    fn fail(&mut self, s: impl Into<String>) {
        self.notice = Some(s.into());
        self.notice_tone = Tone::Error;
        self.notice_since = None;
    }

    /// Whether something on screen moves by itself (a spinner, a countdown), so the loop
    /// redraws without input.
    pub fn animating(&self) -> bool {
        self.stream.is_some() || matches!(self.state.pair, PairState::Offering { .. } | PairState::Waiting { .. })
    }

    // ---- the loop's side: state, jobs, streams ------------------------------------------

    /// Takes in whatever changed since the last frame. Call before every draw.
    pub fn tick(&mut self) -> bool {
        let before = (self.notice.clone(), self.stream.as_ref().map(|s| (s.text.len(), s.reasoning.len(), s.queued, s.node.clone())));
        let mut changed = false;
        if self.watch.has_changed().unwrap_or(false) {
            changed = true;
            let was = self.linked();
            self.state = self.watch.borrow_and_update().clone();
            if !was && self.linked() {
                self.reload();
                if self.view == View::Link && self.auto_link {
                    // `back` is Code when the app was started on a path.
                    self.view = if self.back == View::Code { View::Code } else { View::Chat };
                }
                self.auto_link = false;
                self.say("Linked. Type a message and press Enter.");
            }
        }
        while let Ok(job) = self.jobs.1.try_recv() {
            self.job(job);
            changed = true;
        }
        let streaming = self.stream.is_some();
        self.drain_stream();
        changed |= streaming != self.stream.is_some();
        if self.stream.is_none() && self.last_reload.elapsed() > Duration::from_secs(3) {
            // Conversations from the user's other devices arrive by sync.
            let seen = (self.sessions.len(), self.sessions.first().map(|s| s.updated_ms), self.folders.len());
            self.reload();
            changed |= seen != (self.sessions.len(), self.sessions.first().map(|s| s.updated_ms), self.folders.len());
        }
        // Toasts: note when one shows; take it down when its time is up (questions stay).
        match (&self.notice, &self.notice_since) {
            (None, _) => self.notice_since = None,
            (Some(n), Some((seen, _))) if n == seen => {}
            (Some(n), _) => self.notice_since = Some((n.clone(), Instant::now())),
        }
        if let Some((_, at)) = &self.notice_since
            && self.notice_tone != Tone::Ask
            && at.elapsed().as_millis() > TOAST_MS
        {
            self.notice = None;
            self.notice_since = None;
        }
        let after = (self.notice.clone(), self.stream.as_ref().map(|s| (s.text.len(), s.reasoning.len(), s.queued, s.node.clone())));
        changed || before != after
    }

    fn job(&mut self, job: Job) {
        match job {
            // The link window says it already.
            Job::Offer(Ok(_)) if self.view == View::Link => {}
            Job::Offer(Ok(_)) => self.say("Open this code on a device that's already linked; c copies it."),
            Job::Offer(Err(e)) | Job::Accept(Err(e)) => self.fail(e),
            Job::Accept(Ok(check)) => {
                self.say(format!("Does {} show {}? y yes · n no", check.device, check.sas));
                self.link_check = Some(check);
            }
            Job::Test(id, r) => {
                let line = match r {
                    Ok(t) if t.ok => format!("OK in {} ms · {} model{}", t.latency_ms, t.models.len(), crate::code::plural(t.models.len())),
                    Ok(t) => t.message,
                    Err(e) => e,
                };
                self.say(line.clone());
                self.settings.tests.insert(id, line);
            }
            Job::Models(id, r) => match r {
                Ok(models) if self.models_for.as_deref() == Some(id.as_str()) => self.endpoint_models = models,
                Ok(_) => {}
                Err(e) => self.fail(e),
            },
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
                        if let (Some(t), None) = (s.thinking_since, s.thought_ms) {
                            s.thought_ms = Some(t.elapsed().as_millis() as u64);
                        }
                        truncate_chars(&mut s.text, offset as usize);
                        s.text.push_str(&text);
                    }
                    ChatEvent::Reasoning { offset, text } => {
                        truncate_chars(&mut s.reasoning, offset as usize);
                        s.reasoning.push_str(&text);
                        s.thinking_since.get_or_insert_with(Instant::now);
                    }
                    ChatEvent::Thought { elapsed_ms } => s.thought_ms = Some(elapsed_ms),
                    ChatEvent::Resumed { node, offset } => {
                        s.node = Some(node);
                        truncate_chars(&mut s.text, offset as usize);
                    }
                    ChatEvent::Done { .. } => finished = Some(Ok(())),
                    ChatEvent::Error { kind, message, .. } => finished = Some(Err((kind, message))),
                    ChatEvent::Notice { text } => {
                        self.notice = Some(text);
                        self.notice_tone = Tone::Info;
                    }
                    ChatEvent::Retry { attempt, .. } => {
                        self.notice = Some(format!("Trying again (attempt {attempt})…"));
                        self.notice_tone = Tone::Info;
                    }
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
                self.notice_tone = Tone::Info;
                self.notice = match r {
                    Ok(()) => None,
                    Err((ErrorKind::Cancelled, _)) => Some("Stopped. Ctrl-R continues it.".into()),
                    Err((_, m)) => {
                        self.notice_tone = Tone::Error;
                        Some(m)
                    }
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
            if let Some(i) = self.rows.iter().position(|r| r.chat_id() == Some(h.conversation_id.as_str())) {
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
            thinking_since: None,
            started: Instant::now(),
            thought_ms: None,
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
                View::Settings => {
                    if let Some(f) = &mut self.settings.form {
                        f.fields[f.field].insert(text.trim());
                        f.key_touched |= f.field == settings::KEY_FIELD;
                    }
                }
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
        const OVERLAYS: [View; 4] = [View::Help, View::Models, View::Status, View::Settings];
        if OVERLAYS.contains(&view) && !OVERLAYS.contains(&self.view) {
            self.back = self.view;
        }
        if view == View::Models {
            self.targets = self.core.targets();
            let t = self.effective_target();
            self.picker_target = self.targets.iter().position(|o| Some(&o.target) == t.as_ref()).unwrap_or(0);
            self.picker_models_pane = false;
            self.model_sel = 0;
            self.models_for = None;
            self.load_models();
        }
        if view == View::Settings {
            self.settings.reload(&self.core);
        }
        if view == View::Help {
            self.help_scroll = 0;
        }
        self.auto_link = false;
        self.view = view;
    }

    fn close(&mut self) {
        self.view = if matches!(self.view, View::Help | View::Models | View::Status | View::Settings) { self.back } else { View::Chat };
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
            KeyCode::Char('p') if ctrl => return self.open(View::Settings),
            KeyCode::F(6) => return self.open(View::Settings),
            KeyCode::Char('k') if ctrl => {
                self.show_thinking = !self.show_thinking;
                return self.say(if self.show_thinking { "Showing the reasoning." } else { "Reasoning folded." });
            }
            KeyCode::Char('n') if ctrl => {
                self.open(View::Chat);
                self.current = None;
                self.target = None;
                self.focus = Focus::Input;
                self.scroll = 0;
                return;
            }
            KeyCode::Char('y') if ctrl => return self.copy(),
            _ => {}
        }
        if let Some(id) = self.confirm_delete.take() {
            if k.code == KeyCode::Char('y') {
                let result = match id.strip_prefix("folder:") {
                    Some(folder) => self.core.delete_folder(folder),
                    None => self.core.delete_conversation(&id),
                };
                match result {
                    Ok(()) => {
                        if self.current.as_ref().is_some_and(|c| c.id == id) {
                            self.current = None;
                        }
                        self.reload();
                        self.say("Deleted.");
                    }
                    Err(e) => self.fail(e.message),
                }
            } else {
                self.say("Kept.");
            }
            return;
        }
        match self.view {
            View::Help => match k.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => self.close(),
                KeyCode::Down | KeyCode::Char('j') => self.help_scroll += 1,
                KeyCode::Up | KeyCode::Char('k') => self.help_scroll = self.help_scroll.saturating_sub(1),
                KeyCode::PageDown | KeyCode::Char(' ') => self.help_scroll += 10,
                KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(10),
                _ => {}
            },
            View::Models => self.models_key(k),
            View::Settings => self.settings_key(k, ctrl),
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

    /// The picker's rows: every target, then the models of the selected one (the pool's
    /// catalog, or what the endpoint lists).
    pub fn picker_models(&self) -> Vec<String> {
        match self.targets.get(self.picker_target).map(|t| &t.target) {
            Some(Target::Endpoint { .. }) => self.endpoint_models.clone(),
            _ => self.state.catalog.models.iter().map(|m| m.id.clone()).collect(),
        }
    }

    /// Fetches the models of the endpoint under the picker's cursor.
    fn load_models(&mut self) {
        let Some(Target::Endpoint { id, .. }) = self.targets.get(self.picker_target).map(|t| t.target.clone()) else {
            self.models_for = None;
            return;
        };
        if self.models_for.as_deref() == Some(id.as_str()) {
            return;
        }
        self.models_for = Some(id.clone());
        self.endpoint_models.clear();
        let (core, tx) = (self.core.clone(), self.jobs.0.clone());
        self.core.handle().spawn(async move {
            let r = core.endpoint_models(&id).await.map_err(|e| e.message);
            let _ = tx.send(Job::Models(id, r));
        });
    }

    /// Picks where chats go (and binds the open chat to it).
    fn pick_target(&mut self, t: Target) {
        if let Some(c) = &self.current
            && let Err(e) = self.core.bind_session(&c.id, t.clone())
        {
            return self.fail(e.message);
        }
        self.target = Some(t);
        self.reload();
        self.say(format!("Sending to {} · {}", self.target_label(), self.model_label()));
    }

    fn models_key(&mut self, k: KeyEvent) {
        let nt = self.targets.len();
        let models = self.picker_models();
        let target = self.targets.get(self.picker_target).map(|t| t.target.clone());
        match (k.code, self.picker_models_pane) {
            (KeyCode::Esc | KeyCode::Char('q'), _) => self.close(),
            (KeyCode::Tab | KeyCode::BackTab | KeyCode::Right | KeyCode::Left, _) => {
                self.picker_models_pane = !self.picker_models_pane && !models.is_empty();
                self.model_sel = self.model_sel.min(models.len().saturating_sub(1));
            }
            (KeyCode::Up | KeyCode::Char('k'), false) => {
                self.picker_target = self.picker_target.saturating_sub(1);
                self.model_sel = 0;
                self.load_models();
            }
            (KeyCode::Down | KeyCode::Char('j'), false) => {
                self.picker_target = (self.picker_target + 1).min(nt.saturating_sub(1));
                self.model_sel = 0;
                self.load_models();
            }
            (KeyCode::Up | KeyCode::Char('k'), true) => self.model_sel = self.model_sel.saturating_sub(1),
            (KeyCode::Down | KeyCode::Char('j'), true) => self.model_sel = (self.model_sel + 1).min(models.len().saturating_sub(1)),
            (KeyCode::Char('a') | KeyCode::Backspace, _) => {
                self.model = None;
                self.say("Model: automatic (the first warm one).");
                self.close();
            }
            (KeyCode::Enter, false) => {
                if let Some(t) = target {
                    self.pick_target(t);
                }
                self.close();
            }
            (KeyCode::Enter, true) => {
                let Some(m) = models.get(self.model_sel).cloned() else { return };
                match target {
                    Some(Target::Endpoint { id, .. }) => self.pick_target(Target::Endpoint { id, model: m }),
                    _ => {
                        if !matches!(self.effective_target(), None | Some(Target::Pool)) {
                            self.pick_target(Target::Pool);
                        }
                        self.model = Some(m.clone());
                        self.say(format!("Model: {m}"));
                    }
                }
                self.close();
            }
            _ => {}
        }
    }

    fn settings_key(&mut self, k: KeyEvent, ctrl: bool) {
        if self.settings.form.is_some() {
            return self.form_key(k, ctrl);
        }
        if let Some(id) = self.settings.confirm_delete.take() {
            if k.code == KeyCode::Char('y') {
                match self.core.delete_endpoint(&id, Some(Target::Pool)) {
                    Ok(r) => self.say(format!("Deleted; {} chat{} moved to the Weirdnet pool.", r.rebound, crate::code::plural(r.rebound))),
                    Err(e) => self.fail(e.message),
                }
                if matches!(&self.target, Some(Target::Endpoint { id: t, .. }) if *t == id) {
                    self.target = None;
                }
                self.reload();
            } else {
                self.say("Kept.");
            }
            return;
        }
        let n = self.settings.endpoints.len();
        let sel = self.settings.selected().cloned();
        let edit = |app: &mut Self, e: &bragi_core::EndpointView, input: bragi_core::EndpointInput| {
            match app.core.edit_endpoint(&e.id, input, KeyChange::Keep) {
                Ok(v) => app.say(format!("{}: {}, {}", v.name, if v.enabled { "on" } else { "off" }, mode_words(v.mode))),
                Err(err) => app.say(err.message),
            }
            app.reload();
        };
        match (k.code, sel) {
            (KeyCode::Esc | KeyCode::Char('q'), _) => self.close(),
            (KeyCode::Up | KeyCode::Char('k'), _) => self.settings.sel = self.settings.sel.saturating_sub(1),
            (KeyCode::Down | KeyCode::Char('j'), _) => self.settings.sel = (self.settings.sel + 1).min(n.saturating_sub(1)),
            (KeyCode::Char('a'), _) => self.settings.form = Some(Form::add()),
            (KeyCode::Char('e'), Some(e)) => self.settings.form = Some(Form::edit(&e)),
            (KeyCode::Char(' '), Some(e)) => edit(self, &e, bragi_core::EndpointInput { enabled: !e.enabled, ..settings::input_of(&e) }),
            (KeyCode::Char('m'), Some(e)) => {
                let mode = match e.mode {
                    bragi_core::EndpointMode::Alongside => bragi_core::EndpointMode::Fallback,
                    bragi_core::EndpointMode::Fallback => bragi_core::EndpointMode::Alongside,
                };
                edit(self, &e, bragi_core::EndpointInput { mode, ..settings::input_of(&e) })
            }
            (KeyCode::Char('t'), Some(e)) => {
                let (core, tx) = (self.core.clone(), self.jobs.0.clone());
                self.say(format!("Testing {}…", e.name));
                self.core.handle().spawn(async move {
                    let r = core.test_endpoint(&e.id).await.map_err(|e| e.message);
                    let _ = tx.send(Job::Test(e.id, r));
                });
            }
            (KeyCode::Char('K'), Some(e)) => {
                if let Err(err) = self.core.move_endpoint(&e.id, self.settings.sel.saturating_sub(1)) {
                    self.fail(err.message);
                }
                self.settings.sel = self.settings.sel.saturating_sub(1);
                self.reload();
            }
            (KeyCode::Char('J'), Some(e)) => {
                let to = (self.settings.sel + 1).min(n.saturating_sub(1));
                if let Err(err) = self.core.move_endpoint(&e.id, to) {
                    self.fail(err.message);
                }
                self.settings.sel = to;
                self.reload();
            }
            (KeyCode::Delete | KeyCode::Char('d'), Some(e)) => {
                let bound = self.core.endpoint_sessions(&e.id);
                self.settings.confirm_delete = Some(e.id.clone());
                self.say(format!("Delete {} ({bound} chat{} move to the pool)? y yes · any other key keeps it", e.name, crate::code::plural(bound)));
            }
            (KeyCode::Enter, Some(e)) => {
                let model = e.model.clone().unwrap_or_default();
                self.pick_target(Target::Endpoint { id: e.id.clone(), model });
            }
            _ => {}
        }
    }

    fn form_key(&mut self, k: KeyEvent, ctrl: bool) {
        let Some(form) = &mut self.settings.form else { return };
        match k.code {
            KeyCode::Esc => {
                self.settings.form = None;
                self.say("Not saved.");
            }
            KeyCode::Tab | KeyCode::Down => form.field = (form.field + 1) % settings::FIELDS.len(),
            KeyCode::BackTab | KeyCode::Up => form.field = (form.field + settings::FIELDS.len() - 1) % settings::FIELDS.len(),
            KeyCode::Char('o') if ctrl => form.alongside = !form.alongside,
            KeyCode::Enter if form.field + 1 < settings::FIELDS.len() && !ctrl => form.field += 1,
            KeyCode::Enter | KeyCode::Char('s') if k.code == KeyCode::Enter || ctrl => {
                let form = self.settings.form.take().expect("form");
                let result = match &form.editing {
                    None => self.core.add_endpoint(form.input(true), form.key()),
                    Some(id) => {
                        let enabled = self.settings.endpoints.iter().find(|e| &e.id == id).is_none_or(|e| e.enabled);
                        let key = match (form.key_touched, form.key()) {
                            (false, _) => KeyChange::Keep,
                            (true, Some(k)) => KeyChange::Set(k),
                            (true, None) => KeyChange::Clear,
                        };
                        self.core.edit_endpoint(id, form.input(enabled), key)
                    }
                };
                match result {
                    Ok(v) => {
                        let warn = if v.insecure { " Plain http to another machine: the key and chats can be read on the way." } else { "" };
                        self.say(format!("Saved {} ({}).{warn}", v.name, mode_words(v.mode)));
                        self.reload();
                        self.settings.sel = self.settings.endpoints.iter().position(|e| e.id == v.id).unwrap_or(0);
                    }
                    Err(e) => {
                        self.fail(e.message);
                        self.settings.form = Some(form);
                    }
                }
            }
            _ => {
                let field = form.field;
                if form.fields[field].key(k) && field == settings::KEY_FIELD {
                    form.key_touched = true;
                }
            }
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
                    self.fail(e.message);
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
                Err(e) => self.fail(e.message),
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
            return self.sidebar_key(k);
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

    fn sidebar_key(&mut self, k: KeyEvent) {
        let row = self.rows.get(self.chat_sel).cloned();
        match (k.code, row) {
            (KeyCode::Up | KeyCode::Char('k'), _) => self.chat_sel = self.chat_sel.saturating_sub(1),
            (KeyCode::Down | KeyCode::Char('j'), _) => self.chat_sel = (self.chat_sel + 1).min(self.rows.len().saturating_sub(1)),
            (KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right, Some(Row::Folder { id, open, .. })) => {
                let want_open = match k.code {
                    KeyCode::Left => false,
                    KeyCode::Right => true,
                    _ => !open,
                };
                if want_open {
                    self.closed.remove(&id);
                } else {
                    self.closed.insert(id);
                }
                self.rows = tree::rows(&self.folders, &self.sessions, &self.closed);
            }
            (KeyCode::Enter, Some(Row::Chat { session, .. })) => {
                self.current = self.core.conversation(&session.id).ok();
                self.target = None;
                self.scroll = 0;
                self.focus = Focus::Input;
                if session.unbound {
                    self.say("This chat's endpoint was deleted: Ctrl-O picks where to send it.");
                }
            }
            (KeyCode::Left, Some(Row::Chat { .. })) => {
                // Up to the folder holding it.
                if let Some(i) = (0..self.chat_sel)
                    .rev()
                    .find(|&i| matches!(self.rows[i], Row::Folder { .. }) && self.rows[i].depth() + 1 == self.rows[self.chat_sel].depth())
                {
                    self.chat_sel = i;
                }
            }
            (KeyCode::Delete | KeyCode::Char('d'), Some(Row::Chat { session, .. })) => {
                self.confirm_delete = Some(session.id.clone());
                self.say(format!("Delete \"{}\"? y yes · any other key keeps it", session.title));
            }
            (KeyCode::Delete | KeyCode::Char('d'), Some(Row::Folder { id, name, .. })) => {
                self.confirm_delete = Some(format!("folder:{id}"));
                self.say(format!("Delete the folder \"{name}\"? Its chats move up a level. y yes · any other key keeps it"));
            }
            (KeyCode::Char('n'), _) => {
                self.new_folder_parent = self.sidebar_folder();
                self.prefill("/folder ");
            }
            (KeyCode::Char('m'), Some(Row::Chat { session, .. })) => {
                self.current = self.core.conversation(&session.id).ok();
                self.prefill("/move ");
            }
            (KeyCode::Char('r'), Some(Row::Folder { name, .. })) => self.prefill(&format!("/rename {name}")),
            (KeyCode::Char('?'), _) => self.open(View::Help),
            _ => {}
        }
    }

    fn prefill(&mut self, text: &str) {
        self.input.set(text);
        self.focus = Focus::Input;
    }

    /// The folder the sidebar's cursor is in (a folder row, or the folder above a chat row).
    fn sidebar_folder(&self) -> Option<String> {
        let row = self.rows.get(self.chat_sel)?;
        if let Some(id) = row.folder_id() {
            return Some(id.to_string());
        }
        let Row::Chat { session, .. } = row else { return None };
        session.folder.clone()
    }

    /// Folder commands typed in the message box. `None` when `text` isn't one.
    fn folder_command(&mut self, text: &str) -> Option<Result<String, String>> {
        let (cmd, arg) = text.split_once(' ').map(|(c, a)| (c, a.trim())).unwrap_or((text, ""));
        let err = |e: bragi_core::CoreError| e.message;
        Some(match cmd {
            "/folder" if arg.is_empty() => Err("Name the folder: /folder NAME".into()),
            "/folder" => {
                let parent = self.new_folder_parent.take();
                self.core.create_folder(arg, parent.as_deref()).map(|f| format!("Made the folder {}.", f.name)).map_err(err)
            }
            "/move" => {
                let Some(c) = &self.current else { return Some(Err("Open a chat first.".into())) };
                let folder = match arg {
                    "" | "-" | "/" => None,
                    name => match tree::find_folder(&self.folders, name) {
                        Some(f) => Some(f.id.clone()),
                        None => return Some(Err(format!("No folder called {name}; /folder {name} makes one."))),
                    },
                };
                let id = c.id.clone();
                self.core
                    .set_session_folder(&id, folder.as_deref())
                    .map(|_| if folder.is_some() { format!("Moved to {arg}.") } else { "Moved out of its folder.".into() })
                    .map_err(err)
            }
            "/rename" => {
                let Some(id) = self.rows.get(self.chat_sel).and_then(Row::folder_id).map(str::to_string) else {
                    return Some(Err("Select a folder in the sidebar (Tab) first.".into()));
                };
                self.core.rename_folder(&id, arg).map(|_| format!("Renamed to {arg}.")).map_err(err)
            }
            "/nest" => {
                let Some(id) = self.rows.get(self.chat_sel).and_then(Row::folder_id).map(str::to_string) else {
                    return Some(Err("Select a folder in the sidebar (Tab) first.".into()));
                };
                let parent = match arg {
                    "" | "-" | "/" => None,
                    name => match tree::find_folder(&self.folders, name) {
                        Some(f) => Some(f.id.clone()),
                        None => return Some(Err(format!("No folder called {name}."))),
                    },
                };
                self.core.move_folder(&id, parent.as_deref()).map(|_| "Moved the folder.".to_string()).map_err(err)
            }
            _ => return None,
        })
    }

    /// Sends the input: `/attach PATH` adds a file (text, image or audio) to the next message,
    /// `/detach` drops them; `/folder`, `/move`, `/rename` and `/nest` organise chats.
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
                Err(e) => self.fail(e.message),
            }
            return;
        }
        if text == "/detach" {
            self.attachments.clear();
            self.input.take();
            return self.say("No attachments.");
        }
        if let Some(r) = self.folder_command(&text) {
            match r {
                Ok(n) => {
                    self.input.take();
                    self.reload();
                    self.say(n);
                }
                Err(e) => self.fail(e),
            }
            return;
        }
        let conv = self.current.as_ref().map(|c| c.id.clone());
        let files = std::mem::take(&mut self.attachments);
        let target = self.target.clone();
        match self.core.send_to(conv.as_deref(), &text, self.model.clone(), files.clone(), target) {
            Ok(h) => {
                self.input.take();
                self.notice = None;
                self.follow(h, Purpose::Chat);
            }
            Err(e) => {
                self.attachments = files;
                self.fail(e.message);
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
            Err(e) => self.fail(e.message),
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
                    Err(e) => self.fail(e),
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
            Err(e) => return self.fail(e),
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
                let target = self.effective_target();
                match CodeState::messages(&req).and_then(|m| self.core.complete_to(m, self.model.clone(), target).map_err(|e| e.message)) {
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
            Err(e) => self.fail(e),
        }
    }
}

fn truncate_chars(s: &mut String, chars: usize) {
    if let Some((i, _)) = s.char_indices().nth(chars) {
        s.truncate(i);
    }
}

fn mode_words(m: bragi_core::EndpointMode) -> &'static str {
    match m {
        bragi_core::EndpointMode::Alongside => "offered next to the pool",
        bragi_core::EndpointMode::Fallback => "fallback only",
    }
}
