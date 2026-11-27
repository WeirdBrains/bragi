//! The terminal app's state and what each key does. Drawing is `ui`; behaviour is
//! `bragi-core`'s: this only keeps what the screen shows and calls `Core`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use bragi_core::{
    AppState, Attachment, ChatEvent, ChatHandle, Conversation, Core, ErrorKind, Folder, KeyChange, LinkCheck, MessageState, Mode, PairState,
    SessionSummary, Target, TargetOption, TestReport,
};
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::sync::{mpsc as tmpsc, watch};

use crate::code::{AfterError, CodeState, Command, Phase};
use crate::editor::Editor;
use crate::hit::HitMap;
use crate::settings::{self, Form, Settings};
use crate::tree::{self, Row};
use bragi_core::code::CodeRequest;

mod themes_screen;
pub use themes_screen::{Ask, Confirm, EditState, ThemeScreen, parse_hex};
mod mouse;

pub use mouse::openable;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Chat,
    Link,
    Code,
    Status,
    Models,
    Settings,
    Themes,
    Help,
    /// Past notices, newest first.
    Notifications,
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
    /// A `!command` finished.
    Shell(Result<bragi_core::shell::ShellOutput, String>),
    /// A pool switch or remove finished.
    Pool(Result<String, String>),
    /// A clip was sampled (LF-43).
    Video(Result<Attachment, String>),
    /// A model's video limits were read (the chip, `/video-limits`); with what to say.
    VideoLimits(Result<bragi_core::video::LimitsView, String>, Option<String>),
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
    /// The video limits chip for the model the next message goes to (`/video-limits`, LF-43).
    pub video_limits: Option<bragi_core::video::LimitsView>,
    /// A clip is being sampled.
    pub video_busy: bool,
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
    pub themes: ThemeScreen,
    /// The folder a `/folder` typed after `n` in the sidebar goes in.
    pub new_folder_parent: Option<String>,
    /// Show the reasoning of every answer, not only the title line.
    pub show_thinking: bool,
    /// Hide the chats sidebar (remembered in the profile).
    pub sidebar_collapsed: bool,
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
    /// The file picker, open over code mode.
    pub picker: Option<crate::picker::Picker>,
    pub confirm_delete: Option<String>,
    /// Text to put on the terminal's clipboard (OSC 52), written by the main loop.
    pub clipboard: Vec<String>,
    /// Addresses to open with the system opener, taken by the main loop.
    pub open_urls: Vec<String>,
    pub quit: bool,
    /// What a click does where: rebuilt by every draw ([`crate::hit`]).
    pub hits: HitMap,
    /// The cell the pointer is over, when the terminal reports motion.
    pub hover: Option<(u16, u16)>,
    /// Whether the app wants the terminal's mouse reporting (F8 turns it off so text can be
    /// selected); the main loop makes the terminal match.
    pub mouse: bool,
    last_click: Option<(Instant, crate::hit::Action)>,
    /// The local API server's state, kept up to date in the background (`None` in tests).
    pub serve: Option<crate::serve::Shared>,
    serve_seen: Option<crate::serve::ServeState>,
    jobs: (mpsc::Sender<Job>, mpsc::Receiver<Job>),
    last_reload: Instant,
    /// Prompts sent in chat, for Up and Down.
    pub history: bragi_core::prompt_history::PromptHistory,
    /// Every notice shown, kept (the toasts go away), and the row picked in its list.
    pub notices: bragi_core::notifications::NotificationLog,
    pub notices_sel: usize,
    /// The profile folder (per-project prompt histories live in it).
    home: PathBuf,
    /// A `!command` running: its stop switch.
    pub shell: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Answers whose Thinking is shown the other way round from `show_thinking`
    /// (by conversation id and message index).
    pub thinking_flipped: HashSet<(String, usize)>,
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
            video_limits: None,
            video_busy: false,
            targets: Vec::new(),
            model: None,
            model_sel: 0,
            picker_target: 0,
            picker_models_pane: false,
            endpoint_models: Vec::new(),
            models_for: None,
            settings: Settings::default(),
            themes: ThemeScreen::default(),
            show_thinking: false,
            sidebar_collapsed: false,
            new_folder_parent: None,
            notice: None,
            notice_tone: Tone::Info,
            notice_since: None,
            scroll: 0,
            help_scroll: 0,
            link_input: Editor::default(),
            link_check: None,
            code: CodeState::new(root, home.join("code-undo")),
            picker: None,
            confirm_delete: None,
            clipboard: Vec::new(),
            open_urls: Vec::new(),
            quit: false,
            hits: HitMap::default(),
            hover: None,
            mouse: true,
            last_click: None,
            serve: None,
            serve_seen: None,
            jobs: mpsc::channel(),
            last_reload: Instant::now(),
            history: bragi_core::prompt_history::PromptHistory::open(bragi_core::prompt_history::file_for(home, None)),
            notices: bragi_core::notifications::NotificationLog::open(home),
            notices_sel: 0,
            home: home.to_path_buf(),
            shell: None,
            thinking_flipped: HashSet::new(),
            core,
        };
        app.sidebar_collapsed = app.core.appearance().preference().sidebar_collapsed;
        app.code.history = bragi_core::prompt_history::PromptHistory::open(bragi_core::prompt_history::file_for(home, Some(&app.code.root)));
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
        match &o.context {
            // A file: it is the context.
            Some(file) => match self.code.add(&[file.display().to_string()]) {
                Ok(note) => self.say(&note),
                Err(e) => self.fail(e),
            },
            // A folder: start empty and choose what goes in.
            None => self.open_picker(),
        }
        // A new device links first; the code view waits behind that screen.
        if self.view == View::Link {
            self.back = View::Code;
        } else {
            self.view = View::Code;
        }
    }

    /// Opens the file picker on the code root.
    pub fn open_picker(&mut self) {
        self.picker = Some(crate::picker::Picker::new(&self.code.root));
    }

    fn picker_key(&mut self, k: KeyEvent, ctrl: bool) {
        use crate::picker::Mode;
        let Some(p) = self.picker.as_mut() else { return };
        let filtering = p.mode == Mode::Filter;
        match k.code {
            KeyCode::Up => p.step(-1),
            KeyCode::Down => p.step(1),
            KeyCode::PageUp => p.step(-10),
            KeyCode::PageDown => p.step(10),
            KeyCode::Home => p.first(),
            KeyCode::End => p.last(),
            KeyCode::Char(' ') => self.picker_toggle(),
            KeyCode::Char('u') if ctrl => p.clear_filter(),
            KeyCode::Esc if filtering => p.clear_filter(),
            KeyCode::Esc => self.picker = None,
            KeyCode::Enter if filtering => p.mode = Mode::Browse,
            KeyCode::Enter => self.picker_open(),
            KeyCode::Backspace if filtering => p.backspace(),
            KeyCode::Char(c) if filtering && !ctrl => p.type_char(c),
            KeyCode::Char('q') => self.picker = None,
            KeyCode::Char('k') => p.step(-1),
            KeyCode::Char('j') => p.step(1),
            KeyCode::Char('g') => p.first(),
            KeyCode::Char('G') => p.last(),
            KeyCode::Right | KeyCode::Char('l') => p.expand(),
            KeyCode::Left | KeyCode::Char('h') => p.collapse(),
            KeyCode::Char('/') => p.mode = Mode::Filter,
            KeyCode::Char('c') => {
                self.code.context.clear();
                if let Err(e) = self.code.refresh() {
                    self.fail(e);
                }
            }
            _ => {}
        }
    }

    /// Space: puts the file under the cursor in the context or takes it out; on a folder, all
    /// its files (until the limits say stop).
    fn picker_toggle(&mut self) {
        let Some(p) = self.picker.as_ref() else { return };
        let Some(node) = p.current().cloned() else { return };
        let files: Vec<String> = p.files_under(&node).iter().map(|n| n.path.clone()).collect();
        let all_in = !files.is_empty() && files.iter().all(|f| self.code.has_file(f));
        let mut err = None;
        for f in &files {
            if let Err(e) = self.code.set_file(f, !all_in) {
                err = Some(e);
                break;
            }
        }
        if let Some(e) = err {
            self.fail(e);
        }
        if let Some(p) = self.picker.as_mut()
            && !node.is_dir
            && p.filter.is_empty()
        {
            p.step(1);
        }
    }

    /// Enter: opens or shuts a folder; on a file, puts it in the context and closes the picker.
    fn picker_open(&mut self) {
        let Some(p) = self.picker.as_mut() else { return };
        let Some(node) = p.current().cloned() else { return };
        if node.is_dir {
            return p.toggle_dir();
        }
        match self.code.set_file(&node.path, true) {
            Ok(()) => {
                self.picker = None;
                self.say(format!("Context: {}", self.code.summary()));
            }
            Err(e) => self.fail(e),
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

    /// Where a clip for the next message goes and the model it is sized for: the endpoint's model
    /// when the chat goes to an endpoint, else the picked model or the pool's default.
    fn video_target(&self) -> Result<(Option<Target>, String), String> {
        let target = self.effective_target();
        if let Some(Target::Endpoint { model, .. }) = &target {
            return if model.is_empty() {
                Err("Pick the endpoint's model first: a video is sampled for the model it goes to.".into())
            } else {
                Ok((target.clone(), model.clone()))
            };
        }
        let model = self.model.clone().or_else(|| self.state.catalog.default_model().map(str::to_string));
        model.map(|m| (target, m)).ok_or_else(|| "Pick a model first: a video is sampled for the model it goes to.".into())
    }

    /// Samples a video for the model the next message goes to (LF-43). The frames are sized from
    /// the limits the pool or the endpoint (`/v1/models`) reports, with this device's override for
    /// that model; when they aren't known the refusal says what to set (`/video-limits`). Runs off
    /// the UI thread; [`Job::Video`] brings the attachment back.
    fn attach_video(&mut self, path: &std::path::Path) {
        if self.attachments.iter().any(|a| matches!(a.media, Some(bragi_core::Media::Video(_)))) || self.video_busy {
            return self.fail("One video per message: /detach first.");
        }
        let (target, model) = match self.video_target() {
            Ok(t) => t,
            Err(e) => return self.fail(e),
        };
        let history = self.current.as_ref().map(|c| c.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>()).unwrap_or_default();
        let reserved = bragi_core::video::history_tokens(history);
        let (core, tx, path) = (self.core.clone(), self.jobs.0.clone(), path.to_path_buf());
        self.video_busy = true;
        self.input.take();
        self.say(format!("Reading frames from {}…", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
        self.core.handle().spawn(async move {
            let r = async {
                let plan = core.video_plan(target.as_ref(), &model).await?;
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let prefs = core.video_prefs();
                tokio::task::spawn_blocking(move || {
                    let source = core.video_source(&path)?;
                    core.attach_video_source(&name, &source, &model, &plan, &prefs, reserved)
                })
                .await
                .map_err(|e| bragi_core::CoreError::new(bragi_core::ErrorKind::Other, e.to_string()))?
            }
            .await;
            let _ = tx.send(Job::Video(r.map_err(|e| e.message)));
        });
    }

    /// `/frames`: the frames the model saw of the newest clip in the open chat, with their times.
    fn video_frames(&mut self) {
        let Some(c) = &self.current else { return self.fail("Open a chat with a video first.") };
        let line = match bragi_core::video_play::last_video(c) {
            Some((name, v)) => format!("{}: {}", bragi_core::video_play::bubble(name, v), bragi_core::video_play::frame_list(v).join(" ")),
            None => return self.fail("No video in this chat."),
        };
        self.say(line);
    }

    /// `/play`: the newest clip in the open chat in the system player (the terminal has no inline
    /// player). Says why when the clip isn't on this device.
    fn play_video(&mut self) {
        let Some(c) = &self.current else { return self.fail("Open a chat with a video first.") };
        let Some((name, v)) = bragi_core::video_play::last_video(c) else { return self.fail("No video in this chat.") };
        let name = name.to_string();
        match self.core.playable_clip(v) {
            Ok(path) => match bragi_core::video_play::open_externally(&path) {
                Ok(()) => self.say(format!("Opened {name} in the system player.")),
                Err(e) => self.fail(e.message),
            },
            Err(why) => self.fail(why.message()),
        }
    }

    /// Reads the chip for the composer quietly (after a clip is attached).
    fn refresh_video_chip(&mut self) {
        let Ok((target, model)) = self.video_target() else { return };
        let (core, tx) = (self.core.clone(), self.jobs.0.clone());
        self.core.handle().spawn(async move {
            let r = core.video_limits_view(target.as_ref(), &model).await.map_err(|e| e.message);
            let _ = tx.send(Job::VideoLimits(r, Some(String::new())));
        });
    }

    /// `/store-clips` (show), `/store-clips on|off`: whether clips attached here are copied into
    /// their message so linked devices can play them (this device's video preference).
    fn store_clips_command(&mut self, rest: &str) {
        let mut prefs = self.core.video_prefs();
        match rest {
            "" => {}
            "on" => prefs.store_clip = true,
            "off" => prefs.store_clip = false,
            _ => return self.fail("Use /store-clips on or /store-clips off."),
        }
        if !rest.is_empty()
            && let Err(e) = self.core.set_video_prefs(&prefs)
        {
            return self.fail(e.message);
        }
        let mib = bragi_core::video::MAX_STORED_CLIP_BYTES >> 20;
        self.say(if prefs.store_clip {
            format!("Storing clips: on. Clips you attach here (up to {mib} MiB) go with their message, so your linked devices can play them.")
        } else {
            "Storing clips: off. A clip plays only on the device it was attached on.".to_string()
        });
    }

    /// `/video-limits` (show), `/video-limits key=value …` (set this device's limits for the
    /// model), `/video-limits clear`. The override fills what isn't reported and only tightens
    /// what is.
    fn video_limits_command(&mut self, rest: &str) {
        let (target, model) = match self.video_target() {
            Ok(t) => t,
            Err(e) => return self.fail(e),
        };
        let change = match rest {
            "" => None,
            "clear" | "reset" => Some(bragi_core::video::ModelLimits::default()),
            text => match bragi_core::video::ModelLimits::parse(text) {
                Ok(l) => Some(l),
                Err(e) => return self.fail(e.message),
            },
        };
        let (core, tx) = (self.core.clone(), self.jobs.0.clone());
        self.core.handle().spawn(async move {
            let r = async {
                let mut said = None;
                if let Some(user) = change {
                    let before = core.video_limits_view(target.as_ref(), &model).await?;
                    let ignored = before.ignored(&user);
                    core.set_video_override(&model, user)?;
                    said = Some(if user.is_unknown() {
                        format!("Cleared your video limits for {model}.")
                    } else if ignored.is_empty() {
                        format!("Saved your video limits for {model}.")
                    } else {
                        format!("Saved. {} can only tighten what {model} reports, so it is ignored.", ignored.join(", "))
                    });
                }
                Ok::<_, bragi_core::CoreError>((core.video_limits_view(target.as_ref(), &model).await?, said))
            }
            .await;
            let (view, said) = match r {
                Ok((v, s)) => (Ok(v), s),
                Err(e) => (Err(e.message), None),
            };
            let _ = tx.send(Job::VideoLimits(view, said));
        });
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
        if let Some(c) = &self.current {
            self.current = self.core.conversation(&c.id).ok();
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

    fn toggle_sidebar(&mut self) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        if let Err(e) = self.core.appearance().set_sidebar_collapsed(self.sidebar_collapsed) {
            self.fail(e.to_string());
        } else if self.sidebar_collapsed {
            self.focus = Focus::Input;
            self.say("History bar hidden. F10 shows it.");
        } else {
            self.say("History bar shown.");
        }
    }

    fn start_add_pool(&mut self) {
        let (core, tx) = (self.core.clone(), self.jobs.0.clone());
        self.core.handle().spawn(async move {
            let r = core.add_pool().await.map_err(|e| e.message);
            let _ = tx.send(Job::Offer(r));
        });
        self.open(View::Link);
        self.say("Open this code on a device of the other pool.");
    }

    fn start_switch_pool(&mut self, id: &str) {
        let (core, tx, id) = (self.core.clone(), self.jobs.0.clone(), id.to_string());
        self.core.handle().spawn(async move {
            let r = core.switch_pool(&id).await.map(|p| format!("Switched to {}.", p.name)).map_err(|e| e.message);
            let _ = tx.send(Job::Pool(r));
        });
        self.say("Switching pool…");
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
        if let Some(now) = self.serve.as_ref().and_then(|s| s.lock().ok().map(|g| g.clone()))
            && self.serve_seen.as_ref() != Some(&now)
        {
            changed = true;
            if let crate::serve::ServeState::Failed(m) = &now {
                self.fail(format!("The local API (for editors) didn't start: {m}"));
            }
            self.serve_seen = Some(now);
        }
        if self.watch.has_changed().unwrap_or(false) {
            changed = true;
            let was = self.linked();
            let was_pool = self.state.pool.as_ref().map(|p| p.id.clone());
            self.state = self.watch.borrow_and_update().clone();
            let now_pool = self.state.pool.as_ref().map(|p| p.id.clone());
            if was_pool != now_pool && was_pool.is_some() {
                self.current = None;
                self.reload();
                if let Some(p) = &self.state.pool {
                    self.say(format!("Switched to {}.", p.name));
                }
            }
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
            (Some(n), _) => {
                // Every notice is kept, for the list (F9).
                let level = match self.notice_tone {
                    Tone::Info => bragi_core::notifications::Level::Info,
                    Tone::Ok => bragi_core::notifications::Level::Ok,
                    Tone::Ask => bragi_core::notifications::Level::Ask,
                    Tone::Error => bragi_core::notifications::Level::Error,
                };
                self.notices.push(level, n);
                self.notice_since = Some((n.clone(), Instant::now()));
                changed = true;
            }
        }
        // A toast under the pointer stays (so it can be read); its clock restarts after.
        if self.toast_hovered()
            && let Some((_, at)) = &mut self.notice_since
        {
            *at = Instant::now();
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
            Job::Video(r) => {
                self.video_busy = false;
                match r {
                    Ok(a) => {
                        let summary = match &a.media {
                            Some(bragi_core::Media::Video(v)) => v.summary.clone(),
                            _ => a.name.clone(),
                        };
                        self.say(format!("Attached to the next message: {summary}"));
                        self.attachments.push(a);
                        self.refresh_video_chip();
                    }
                    Err(e) => {
                        self.fail(e);
                        self.video_limits_command("");
                    }
                }
            }
            Job::VideoLimits(r, said) => match r {
                Ok(view) if said.as_deref() == Some("") => self.video_limits = Some(view),
                Ok(view) => {
                    let line = match said {
                        Some(s) => format!("{s} {}", view.chip()),
                        None => format!(
                            "{} · reported: {} · yours: {} · /video-limits key=value… (keys: {}) or /video-limits clear",
                            view.chip(),
                            view.reported.describe(),
                            view.user.describe(),
                            bragi_core::video::OVERRIDE_KEYS.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(" ")
                        ),
                    };
                    self.video_limits = Some(view);
                    self.say(line);
                }
                Err(e) => self.fail(e),
            },
            Job::Models(id, r) => match r {
                Ok(models) if self.models_for.as_deref() == Some(id.as_str()) => self.endpoint_models = models,
                Ok(_) => {}
                Err(e) => self.fail(e),
            },
            Job::Pool(Ok(msg)) => {
                self.current = None;
                self.reload();
                self.say(msg);
            }
            Job::Pool(Err(e)) => self.fail(e),
            Job::Shell(r) => {
                self.shell = None;
                match r {
                    Ok(out) => {
                        let ok = out.ok();
                        let line = format!("$ {} — {}", out.command, out.status());
                        self.record_shell(out);
                        if ok { self.say(line) } else { self.fail(line) }
                    }
                    Err(e) => self.fail(e),
                }
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
                let (reasoning, body) =
                    if s.reasoning.trim().is_empty() { bragi_core::think::split(&s.text) } else { (s.reasoning.clone(), s.text.clone()) };
                let note = self.code.answered(&body);
                let mut text = bragi_core::code::prompt::explanation(&body);
                if !self.code.diffs.is_empty() {
                    let files: Vec<String> = self
                        .code
                        .diffs
                        .iter()
                        .map(|d| {
                            let (a, r) = d.stats();
                            format!("`{}` (+{a} -{r})", d.path)
                        })
                        .collect();
                    text = format!("{}\n\nProposed changes: {}.", text.trim_end(), files.join(", "));
                }
                if text.trim().is_empty() {
                    text = note.clone();
                }
                self.record_code_answer(&s, text, reasoning, MessageState::Complete);
                self.scroll = 0;
                self.say(note);
            }
            (Purpose::Code, Err((ErrorKind::Cancelled, _))) => {
                self.code.failed();
                let text = if s.text.trim().is_empty() { "(stopped)".to_string() } else { s.text.clone() };
                let reasoning = s.reasoning.clone();
                self.record_code_answer(&s, text, reasoning, MessageState::Stopped);
                self.say("Stopped.".to_string());
            }
            (Purpose::Code, Err((_, m))) => {
                self.code.failed();
                match self.code.after_error(&m) {
                    AfterError::Retry(req, why) => match self.code_send(req) {
                        Ok(()) => self.say(format!("{why}…")),
                        Err(e) => self.fail(e),
                    },
                    AfterError::Fail(m) => {
                        let reasoning = s.reasoning.clone();
                        self.record_code_answer(&s, m.clone(), reasoning, MessageState::Failed);
                        self.fail(m)
                    }
                }
            }
        }
    }

    /// Saves a finished code turn (the line typed and the answer, both marked code) in the
    /// open conversation, or a new one, and shows it.
    fn record_code_answer(&mut self, s: &Stream, text: String, reasoning: String, state: MessageState) {
        let ask = std::mem::take(&mut self.code.asked_line);
        let mut answer = bragi_core::ChatMessage::new(bragi_core::Role::Assistant, text, Mode::Code);
        answer.thinking_ms = s.thought_ms.or_else(|| s.thinking_since.map(|t| t.elapsed().as_millis() as u64)).filter(|_| !reasoning.is_empty());
        answer.reasoning = reasoning;
        answer.model = s.model.clone();
        answer.node = s.node.clone();
        answer.state = state;
        let mut turn = Vec::new();
        if !ask.trim().is_empty() {
            turn.push(bragi_core::ChatMessage::new(bragi_core::Role::User, ask, Mode::Code));
        }
        turn.push(answer);
        self.record(turn);
    }

    /// A shell run, saved in the conversation as a turn of the mode it was run in.
    fn record_shell(&mut self, out: bragi_core::shell::ShellOutput) {
        let t = out.transcript();
        let fence = if t.contains("```") { "~~~~" } else { "```" };
        let text = format!("{fence}console\n{t}{fence}");
        let msg = bragi_core::ChatMessage::new(bragi_core::Role::User, text, self.mode());
        self.record(vec![msg]);
    }

    fn record(&mut self, messages: Vec<bragi_core::ChatMessage>) {
        let conv = self.current.as_ref().map(|c| c.id.clone());
        match self.core.record(conv.as_deref(), messages) {
            Ok(id) => {
                self.current = self.core.conversation(&id).ok();
                self.reload();
                if let Some(i) = self.rows.iter().position(|r| r.chat_id() == Some(id.as_str())) {
                    self.chat_sel = i;
                }
                self.scroll = 0;
            }
            Err(e) => self.fail(e.message),
        }
    }

    /// Chat or code: the conversation is the same in both.
    pub fn mode(&self) -> Mode {
        let base = if matches!(self.view, View::Help | View::Models | View::Status | View::Settings | View::Themes | View::Notifications) {
            self.back
        } else {
            self.view
        };
        if base == View::Code { Mode::Code } else { Mode::Chat }
    }

    /// Switches to `mode` (the other one when `None`), keeping the conversation, its
    /// transcript, the draft and the context.
    pub fn set_mode(&mut self, mode: Option<Mode>) {
        let mode = mode.unwrap_or_else(|| self.mode().other());
        self.view = if mode == Mode::Code { View::Code } else { View::Chat };
        self.auto_link = false;
        self.focus = Focus::Input;
        self.say(match mode {
            Mode::Code => "Code mode: files as context, edits as diffs, !shell. Tab goes back to chat.",
            Mode::Chat => "Chat mode: talk and plan. Tab goes to code.",
        });
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

    /// Handles one terminal event; `false` when nothing on screen changes because of it.
    pub fn on_event(&mut self, ev: Event) -> bool {
        match ev {
            Event::Mouse(m) => return self.on_mouse(m),
            Event::Resize(..) => {
                self.hover = None;
            }
            _ => {}
        }
        match ev {
            Event::Key(k) if k.kind != KeyEventKind::Release => self.on_key(k),
            Event::Paste(text) => match self.view {
                View::Link => self.link_input.insert(text.trim()),
                View::Themes => self.themes_paste(&text),
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
        true
    }

    fn open(&mut self, view: View) {
        if view == self.view {
            return;
        }
        const OVERLAYS: [View; 6] = [View::Help, View::Models, View::Status, View::Settings, View::Themes, View::Notifications];
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
        if view == View::Themes {
            self.open_themes();
        }
        if view == View::Notifications {
            self.notices_sel = 0;
            self.notices.seen();
        }
        self.auto_link = false;
        self.view = view;
    }

    fn close(&mut self) {
        self.view = if matches!(self.view, View::Help | View::Models | View::Status | View::Settings | View::Themes | View::Notifications) {
            self.back
        } else {
            View::Chat
        };
    }

    pub fn on_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        // The file picker takes every key but the ones that quit.
        if self.picker.is_some() && self.view == View::Code && !(ctrl && matches!(k.code, KeyCode::Char('q' | 'c'))) {
            return self.picker_key(k, ctrl);
        }
        if ctrl && k.code == KeyCode::Char('f') && self.view == View::Code && self.code.phase == Phase::Compose {
            return self.open_picker();
        }
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
            KeyCode::F(9) => return if self.view == View::Notifications { self.close() } else { self.open(View::Notifications) },
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
            KeyCode::Char('g') if ctrl => return self.open(View::Themes),
            KeyCode::F(7) => return self.open(View::Themes),
            KeyCode::F(10) => return self.toggle_sidebar(),
            KeyCode::Char('k') if ctrl => {
                self.show_thinking = !self.show_thinking;
                // Ctrl-K sets every Thinking one way; clicks then flip single ones.
                self.thinking_flipped.clear();
                return self.say(if self.show_thinking { "Showing the reasoning." } else { "Reasoning folded." });
            }
            KeyCode::Char('n') if ctrl => {
                // A new conversation in the mode you are in.
                if self.mode() == Mode::Code && self.code.phase == Phase::Compose {
                    self.view = View::Code;
                } else {
                    self.open(View::Chat);
                }
                self.current = None;
                self.target = None;
                self.focus = Focus::Input;
                self.scroll = 0;
                return;
            }
            KeyCode::Char('y') if ctrl => return self.copy(),
            KeyCode::F(8) => {
                self.mouse = !self.mouse;
                self.hover = None;
                return self.say(if self.mouse {
                    "Mouse on: click and scroll. F8 turns it off so text can be selected."
                } else {
                    "Mouse off: select text as usual. F8 turns it back on."
                });
            }
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
            View::Themes => self.themes_key(k),
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
            View::Notifications => self.notices_key(k),
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
            // A rollout's pinnable versions follow its name.
            _ => {
                self.state.catalog.models.iter().flat_map(|m| std::iter::once(m.id.clone()).chain(m.versions.iter().map(|v| v.id.clone()))).collect()
            }
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
        if let Some(id) = self.settings.confirm_delete_pool.take() {
            if k.code == KeyCode::Char('y') {
                let (core, tx) = (self.core.clone(), self.jobs.0.clone());
                self.core.handle().spawn(async move {
                    let r = core.remove_pool(&id).await.map(|_| "Removed the pool.".into()).map_err(|e| e.message);
                    let _ = tx.send(Job::Pool(r));
                });
                self.say("Removing the pool…");
            } else {
                self.say("Kept.");
            }
            return;
        }
        let n = self.settings.row_count();
        let sel = self.settings.selected().cloned();
        let pool = self.settings.selected_pool().cloned();
        let edit = |app: &mut Self, e: &bragi_core::EndpointView, input: bragi_core::EndpointInput| {
            match app.core.edit_endpoint(&e.id, input, KeyChange::Keep) {
                Ok(v) => app.say(format!("{}: {}, {}", v.name, if v.enabled { "on" } else { "off" }, mode_words(v.mode))),
                Err(err) => app.say(err.message),
            }
            app.reload();
        };
        match (k.code, sel, pool) {
            (KeyCode::Esc | KeyCode::Char('q'), _, _) => self.close(),
            (KeyCode::Up | KeyCode::Char('k'), _, _) => self.settings.sel = self.settings.sel.saturating_sub(1),
            (KeyCode::Down | KeyCode::Char('j'), _, _) => self.settings.sel = (self.settings.sel + 1).min(n.saturating_sub(1)),
            (KeyCode::Char('p'), _, _) => self.start_add_pool(),
            (KeyCode::Enter, _, Some(p)) if !p.active => self.start_switch_pool(&p.id),
            (KeyCode::Enter, _, Some(p)) => self.say(format!("{} is already the active pool.", p.name)),
            (KeyCode::Char('d'), _, Some(p)) => {
                self.settings.confirm_delete_pool = Some(p.id.clone());
                self.say(format!("Forget {} and its history on this device? y yes · any other key keeps it", p.name));
            }
            (KeyCode::Char('a'), _, _) => self.settings.form = Some(Form::add()),
            (KeyCode::Char('e'), Some(e), _) => self.settings.form = Some(Form::edit(&e)),
            (KeyCode::Char(' '), Some(e), _) => edit(self, &e, bragi_core::EndpointInput { enabled: !e.enabled, ..settings::input_of(&e) }),
            (KeyCode::Char('m'), Some(e), _) => {
                let mode = match e.mode {
                    bragi_core::EndpointMode::Alongside => bragi_core::EndpointMode::Fallback,
                    bragi_core::EndpointMode::Fallback => bragi_core::EndpointMode::Alongside,
                };
                edit(self, &e, bragi_core::EndpointInput { mode, ..settings::input_of(&e) })
            }
            (KeyCode::Char('t'), Some(e), _) => {
                let (core, tx) = (self.core.clone(), self.jobs.0.clone());
                self.say(format!("Testing {}…", e.name));
                self.core.handle().spawn(async move {
                    let r = core.test_endpoint(&e.id).await.map_err(|e| e.message);
                    let _ = tx.send(Job::Test(e.id, r));
                });
            }
            (KeyCode::Char('K'), Some(e), _) => {
                let i = self.settings.sel - self.settings.pools.len();
                let to = i.saturating_sub(1);
                if let Err(err) = self.core.move_endpoint(&e.id, to) {
                    self.fail(err.message);
                }
                self.settings.sel = self.settings.pools.len() + to;
                self.reload();
            }
            (KeyCode::Char('J'), Some(e), _) => {
                let i = self.settings.sel - self.settings.pools.len();
                let last = self.settings.endpoints.len().saturating_sub(1);
                let to = (i + 1).min(last);
                if let Err(err) = self.core.move_endpoint(&e.id, to) {
                    self.fail(err.message);
                }
                self.settings.sel = self.settings.pools.len() + to;
                self.reload();
            }
            (KeyCode::Delete | KeyCode::Char('d'), Some(e), _) => {
                let bound = self.core.endpoint_sessions(&e.id);
                self.settings.confirm_delete = Some(e.id.clone());
                self.say(format!("Delete {} ({bound} chat{} move to the pool)? y yes · any other key keeps it", e.name, crate::code::plural(bound)));
            }
            (KeyCode::Enter, Some(e), _) => {
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
        // Tab (or Shift-Tab) switches to code mode; Ctrl-B moves between the chats and the box.
        if k.code == KeyCode::Tab || k.code == KeyCode::BackTab {
            return self.set_mode(Some(Mode::Code));
        }
        if k.code == KeyCode::Char('b') && ctrl {
            if self.sidebar_collapsed && self.focus == Focus::Input {
                self.toggle_sidebar();
                self.focus = Focus::Chats;
            } else {
                self.focus = if self.focus == Focus::Input { Focus::Chats } else { Focus::Input };
            }
            return;
        }
        if k.code == KeyCode::Char('r') && ctrl {
            return self.continue_answer();
        }
        if k.code == KeyCode::Esc {
            if let Some(s) = &self.stream {
                self.core.cancel(&s.request_id);
            } else if let Some(stop) = &self.shell {
                stop.store(true, std::sync::atomic::Ordering::Relaxed);
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
            KeyCode::Up if !ctrl && !alt && self.input.on_first_line() => {
                if let Some(t) = self.history.up(self.input.text()) {
                    self.input.set(&t);
                } else if self.input.is_empty() {
                    self.scroll += 1;
                }
            }
            KeyCode::Down if !ctrl && !alt && self.input.on_last_line() => {
                if let Some(t) = self.history.down() {
                    self.input.set(&t);
                } else if self.input.is_empty() {
                    self.scroll = self.scroll.saturating_sub(1);
                }
            }
            _ => {
                if matches!(k.code, KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete) {
                    self.history.reset();
                }
                self.input.key(k);
            }
        }
    }

    /// The notifications list: Up/Down pick, c clears, Esc closes.
    fn notices_key(&mut self, k: KeyEvent) {
        let n = self.notices.entries().len();
        match k.code {
            KeyCode::Esc | KeyCode::Char('q') => self.close(),
            KeyCode::Up | KeyCode::Char('k') => self.notices_sel = self.notices_sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.notices_sel = (self.notices_sel + 1).min(n.saturating_sub(1)),
            KeyCode::PageUp => self.notices_sel = self.notices_sel.saturating_sub(10),
            KeyCode::PageDown => self.notices_sel = (self.notices_sel + 10).min(n.saturating_sub(1)),
            KeyCode::Home | KeyCode::Char('g') => self.notices_sel = 0,
            KeyCode::End | KeyCode::Char('G') => self.notices_sel = n.saturating_sub(1),
            KeyCode::Char('y') | KeyCode::Enter => {
                // Newest first on screen.
                if let Some(e) = self.notices.entries().iter().rev().nth(self.notices_sel) {
                    self.clipboard.push(e.text.clone());
                }
            }
            KeyCode::Char('c') => {
                self.notices.clear();
                self.notices_sel = 0;
            }
            _ => {}
        }
    }

    /// Whether the pointer is over the toast.
    fn toast_hovered(&self) -> bool {
        let Some((x, y)) = self.hover else { return false };
        self.hits.hits().iter().any(|h| h.action == crate::hit::Action::DismissToast && crate::hit::contains(h.rect, x, y))
    }

    /// Runs a `!command` in `dir` in the background; its output comes back as [`Job::Shell`].
    fn run_shell(&mut self, command: String, dir: PathBuf) {
        if self.shell.is_some() {
            return self.say("A command is still running; Esc stops it.");
        }
        let tx = self.jobs.0.clone();
        let stop = bragi_core::shell::spawn(command.clone(), dir, bragi_core::shell::DEFAULT_TIMEOUT, move |r| {
            let _ = tx.send(Job::Shell(r.map_err(|e| e.message)));
        });
        self.shell = Some(stop);
        self.say(format!("Running {command}…"));
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
                    return Some(Err("Select a folder in the sidebar (Ctrl-B) first.".into()));
                };
                self.core.rename_folder(&id, arg).map(|_| format!("Renamed to {arg}.")).map_err(err)
            }
            "/train" => {
                let Some(c) = &self.current else { return Some(Err("Open a chat first.".into())) };
                let on = match arg {
                    "" | "on" => true,
                    "off" => false,
                    _ => return Some(Err("/train marks the open chat for training; /train off withdraws it.".into())),
                };
                let id = c.id.clone();
                let done = self.core.set_training(&id, on).map_err(err);
                if done.is_ok() {
                    self.current = self.core.conversation(&id).ok();
                }
                done.map(|_| {
                    if on {
                        "Marked for training: your Ymir nodes may export it as training data, on your own devices, without system prompts, attachments, tool results or reasoning. /train off withdraws it.".into()
                    } else {
                        "No longer used for training.".into()
                    }
                })
            }
            "/nest" => {
                let Some(id) = self.rows.get(self.chat_sel).and_then(Row::folder_id).map(str::to_string) else {
                    return Some(Err("Select a folder in the sidebar (Ctrl-B) first.".into()));
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

    fn pool_command(&mut self, rest: &str) {
        let mut it = rest.split_whitespace();
        let sub = it.next().unwrap_or("list");
        match sub {
            "list" | "" => {
                let pools = self.core.pools();
                if pools.is_empty() {
                    self.say("No Ymir pools yet. /pool add links another one.");
                } else {
                    let line =
                        pools.iter().map(|p| format!("{}{} ({})", if p.active { "* " } else { "" }, p.name, p.id)).collect::<Vec<_>>().join(" · ");
                    self.say(line);
                }
            }
            "add" => self.start_add_pool(),
            "switch" | "use" => match it.next() {
                Some(id) => self.start_switch_pool(id),
                None => self.fail("usage: /pool switch NAME"),
            },
            "rename" => {
                let id = it.next();
                let name = it.collect::<Vec<_>>().join(" ");
                match (id, name.is_empty()) {
                    (Some(id), false) => match self.core.rename_pool(id, &name) {
                        Ok(p) => {
                            self.reload();
                            self.say(format!("Renamed to {}.", p.name));
                        }
                        Err(e) => self.fail(e.message),
                    },
                    _ => self.fail("usage: /pool rename NAME NEW-NAME"),
                }
            }
            "remove" | "rm" | "delete" => match it.next() {
                Some(id) => {
                    let (core, tx, id) = (self.core.clone(), self.jobs.0.clone(), id.to_string());
                    self.core.handle().spawn(async move {
                        let r = core.remove_pool(&id).await.map(|_| "Removed the pool.".into()).map_err(|e| e.message);
                        let _ = tx.send(Job::Pool(r));
                    });
                    self.say("Removing the pool…");
                }
                None => self.fail("usage: /pool remove NAME"),
            },
            _ => self.fail("usage: /pool list | add | switch NAME | rename NAME NEW | remove NAME"),
        }
    }

    /// Sends the input: `/attach PATH` adds a file (text, image or audio) to the next message,
    /// `/detach` drops them; `/folder`, `/move`, `/rename` and `/nest` organise chats; `/train`
    /// marks the open chat for training (`/train off` withdraws it).
    pub fn send(&mut self) {
        if self.stream.is_some() {
            return self.say("An answer is still coming; Esc stops it.");
        }
        let text = self.input.text().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.history.push(&text);
        if bragi_core::shell::is_shell_line(&text) {
            let Some(cmd) = bragi_core::shell::parse(&text).map(str::to_string) else {
                return self.fail("usage: !COMMAND or /shell COMMAND");
            };
            self.input.take();
            let dir = self.code.root.clone();
            return self.run_shell(cmd, dir);
        }
        if text == "/notifications" || text == "/notices" {
            self.input.take();
            return self.open(View::Notifications);
        }
        if let Ok(crate::code::Command::Mode(m)) = crate::code::mode_command(&text) {
            self.input.take();
            return self.set_mode(m);
        }
        if let Some(path) = text.strip_prefix("/attach ") {
            let path = crate::code::expand(path.trim());
            if bragi_core::video::is_video_path(&path) {
                return self.attach_video(&path);
            }
            match Attachment::from_path(&path) {
                Ok(a) => {
                    self.say(format!("Attached {} to the next message.", a.name));
                    self.attachments.push(a);
                    self.input.take();
                }
                Err(e) => self.fail(e.message),
            }
            return;
        }
        if text == "/frames" {
            self.input.take();
            return self.video_frames();
        }
        if text == "/play" {
            self.input.take();
            return self.play_video();
        }
        if text == "/store-clips" || text.starts_with("/store-clips ") {
            self.input.take();
            return self.store_clips_command(text.trim_start_matches("/store-clips").trim());
        }
        if text == "/video-limits" || text.starts_with("/video-limits ") {
            self.input.take();
            return self.video_limits_command(text.trim_start_matches("/video-limits").trim());
        }
        if text == "/detach" {
            self.attachments.clear();
            self.input.take();
            return self.say("No attachments.");
        }
        if text == "/sidebar" || text == "/history" {
            self.input.take();
            return self.toggle_sidebar();
        }
        if let Some(rest) = text.strip_prefix("/pool") {
            self.input.take();
            return self.pool_command(rest.trim());
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
                KeyCode::Tab | KeyCode::BackTab => self.set_mode(Some(Mode::Chat)),
                KeyCode::Esc if self.shell.is_some() => {
                    if let Some(stop) = &self.shell {
                        stop.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                KeyCode::Up if !ctrl && !alt && self.input.on_first_line() => {
                    if let Some(t) = self.code.history.up(self.input.text()) {
                        self.input.set(&t);
                    } else if self.input.is_empty() {
                        self.scroll += 1;
                    }
                }
                KeyCode::Down if !ctrl && !alt && self.input.on_last_line() => {
                    if let Some(t) = self.code.history.down() {
                        self.input.set(&t);
                    } else if self.input.is_empty() {
                        self.scroll = self.scroll.saturating_sub(1);
                    }
                }
                KeyCode::Esc if self.input.is_empty() => self.set_mode(Some(Mode::Chat)),
                KeyCode::Esc => {
                    self.input.take();
                }
                KeyCode::Enter if alt => self.input.insert("\n"),
                KeyCode::Char('j') if ctrl => self.input.insert("\n"),
                KeyCode::Enter => self.code_submit(),
                KeyCode::Char('?') if self.input.is_empty() => self.open(View::Help),
                KeyCode::Char('f') if self.input.is_empty() && !ctrl && !alt => self.open_picker(),
                _ => {
                    self.input.key(k);
                }
            },
        }
    }

    /// The model a code request goes to: the endpoint's, the picked one, or the catalog's default.
    pub fn code_model(&self) -> Option<String> {
        if let Some(Target::Endpoint { model, .. }) = self.effective_target() {
            return (!model.is_empty()).then_some(model);
        }
        self.model.clone().or_else(|| self.state.catalog.default_model().map(str::to_string))
    }

    /// Sends an already fitted code request and follows its answer.
    fn code_send(&mut self, req: CodeRequest) -> Result<(), String> {
        let target = self.effective_target();
        let h =
            CodeState::messages(&req, &self.code.turns).and_then(|m| self.core.complete_to(m, self.model.clone(), target).map_err(|e| e.message))?;
        self.code.start(req);
        self.follow(h, Purpose::Code);
        Ok(())
    }

    /// Runs the code-mode line in the input box.
    pub fn code_submit(&mut self) {
        let line = self.input.text().to_string();
        let cmd = match self.code.parse(&line) {
            Ok(c) => c,
            Err(e) => return self.fail(e),
        };
        self.code.history.push(&line);
        self.input.take();
        let note = match cmd {
            Command::Nothing => return,
            Command::Shell(c) => {
                let dir = self.code.root.clone();
                return self.run_shell(c, dir);
            }
            Command::Notifications => return self.open(View::Notifications),
            Command::Mode(m) => return self.set_mode(m),
            Command::Add(paths) => self.code.add(&paths),
            Command::Drop(what) => self.code.drop_path(&what),
            Command::Pick => return self.open_picker(),
            Command::Clear => {
                self.code.context.clear();
                self.code.refresh().map(|_| "Context cleared.".to_string())
            }
            Command::Root(dir) => {
                let dir = crate::code::expand(&dir);
                let dir = if dir.is_absolute() { dir } else { self.code.root.join(dir) };
                if dir.exists() && !dir.is_dir() {
                    Err(format!("{} isn't a folder", dir.display()))
                } else {
                    let home = self.home.clone();
                    self.code.set_root(dir, &home)
                }
            }
            Command::Undo { force } => self.code.undo(force),
            Command::Ask(req) => {
                if self.stream.is_some() {
                    return self.say("An answer is still coming; Esc stops it.");
                }
                let title = bragi_core::code::prompt::title(&req);
                let model = self.code_model();
                let listed = model.as_deref().and_then(|m| self.state.catalog.context_for(m));
                // The conversation so far goes along (a plan made in chat, earlier code turns),
                // within a third of the prompt's room.
                let room = model.as_deref().and_then(|m| self.code.budget_for(m, listed)).map_or(8_000, |b| b.prompt_tokens() / 3);
                let (turns, reserve) = match &self.current {
                    Some(c) => crate::code::conversation_turns(&c.messages, room),
                    None => (Vec::new(), 0),
                };
                self.code.turns = turns;
                self.code.asked_line = line.trim().to_string();
                let (req, left_out) = match model.as_deref() {
                    Some(m) => match self.code.prepare(req, m, listed, reserve) {
                        Ok(x) => x,
                        Err(e) => return self.fail(e),
                    },
                    None => (req, String::new()),
                };
                self.code_send(req).map(|()| {
                    let asking = format!("{title} — asking {}…", self.model_label());
                    if left_out.is_empty() { asking } else { format!("{left_out}. {asking}") }
                })
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
