//! Code mode for the Flutter app, and its desktop link: the phone pairs with a desktop running
//! `bragi linker` and works on the desktop's projects; a computer can also work on a local
//! folder. Every function is one `bragi_core::link` call; the types are flat copies for Dart.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use bragi_core::code::FileDiff;
use bragi_core::code::diff::LineTag;
use bragi_core::link::{CodeSession, Project, Route, ShellFrame};
use bragi_core::notifications::{Level, NotificationLog};
use bragi_core::prompt_history::{PromptHistory, file_for};
use bragi_core::{ChatMessage, Mode, Role};
use flutter_rust_bridge::frb;

use super::bragi::{ChatStarted, register, running_parts};
use crate::frb_generated::StreamSink;

fn session() -> &'static tokio::sync::Mutex<Option<CodeSession>> {
    static S: OnceLock<tokio::sync::Mutex<Option<CodeSession>>> = OnceLock::new();
    S.get_or_init(|| tokio::sync::Mutex::new(None))
}

fn shells() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static S: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn err(e: bragi_core::CoreError) -> String {
    e.message
}

/// Runs `f` on the core's tokio runtime (the link and the shell need one).
async fn on_core<T: Send + 'static>(f: impl std::future::Future<Output = Result<T, String>> + Send + 'static) -> Result<T, String> {
    let (core, _, _) = running_parts()?;
    core.handle().spawn(f).await.map_err(|e| e.to_string())?
}

// ---- types -------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RootView {
    pub name: String,
    pub path: String,
}

/// A paired desktop.
#[derive(Debug, Clone)]
pub struct DesktopView {
    pub id: String,
    pub label: String,
    pub roots: Vec<RootView>,
    /// Shell commands wait for a yes on the desktop.
    pub confirm_shell: bool,
    pub direct: Vec<String>,
    /// Reachable through Weirdnet (the relay).
    pub weirdnet: bool,
    pub paired_ms: i64,
}

fn desktop_view(d: bragi_core::link::DesktopView) -> DesktopView {
    DesktopView {
        id: d.id,
        label: d.label,
        roots: d.roots.into_iter().map(|r| RootView { name: r.name, path: r.path }).collect(),
        confirm_shell: d.confirm_shell,
        direct: d.direct,
        weirdnet: d.weirdnet,
        paired_ms: d.paired_ms,
    }
}

/// The open project.
#[derive(Debug, Clone)]
pub struct ProjectView {
    pub name: String,
    /// Where it is: a path here, or `PATH on DESKTOP`.
    pub place: String,
    /// The desktop it is on (none: this device).
    pub desktop_id: Option<String>,
    /// The project folder relative to the shared root (where listing starts).
    pub base: String,
}

#[derive(Debug, Clone)]
pub struct OpenedView {
    pub project: ProjectView,
    /// What was set up (a new folder, starter files); empty when nothing was.
    pub note: String,
}

#[derive(Debug, Clone)]
pub struct EntryView {
    pub name: String,
    /// Relative to the root, for `code_set_context`.
    pub path: String,
    pub dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct ContextFileView {
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct ContextView {
    pub targets: Vec<String>,
    pub files: Vec<ContextFileView>,
    /// `path: why` for each file left out.
    pub skipped: Vec<String>,
    pub total_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct DiffLineView {
    /// `context`, `added` or `removed`.
    pub tag: String,
    pub text: String,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct HunkView {
    pub header: String,
    pub lines: Vec<DiffLineView>,
    pub accepted: bool,
}

#[derive(Debug, Clone)]
pub struct FileDiffView {
    pub path: String,
    pub created: bool,
    pub added: u32,
    pub removed: u32,
    pub hunks: Vec<HunkView>,
}

/// The changes an answer proposes, for review.
#[derive(Debug, Clone)]
pub struct ReviewView {
    pub explanation: String,
    pub files: Vec<FileDiffView>,
    /// Why there is nothing to review (no change proposed, or the answer couldn't be read).
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CodeAskView {
    pub started: ChatStarted,
    /// Files left out to fit the model's window (empty when none were).
    pub note: String,
}

#[derive(Debug, Clone)]
pub struct AnsweredView {
    /// The conversation the code turn was saved in.
    pub conversation_id: String,
    pub review: ReviewView,
}

#[derive(Debug, Clone)]
pub struct AppliedView {
    pub files: Vec<String>,
}

/// One piece of a shell run: `kind` is `waiting` (the desktop is asking its user), `alive`
/// (still running, quietly), `out`,
/// `err`, `exit` or `denied`.
#[derive(Debug, Clone, Default)]
pub struct ShellEventView {
    pub kind: String,
    pub text: String,
    pub code: Option<i32>,
    pub stopped: bool,
    pub timed_out: bool,
    pub elapsed_ms: u64,
}

fn shell_view(f: ShellFrame) -> ShellEventView {
    match f {
        ShellFrame::Waiting => ShellEventView { kind: "waiting".into(), ..Default::default() },
        ShellFrame::Alive => ShellEventView { kind: "alive".into(), ..Default::default() },
        ShellFrame::Out { text } => ShellEventView { kind: "out".into(), text, ..Default::default() },
        ShellFrame::Err { text } => ShellEventView { kind: "err".into(), text, ..Default::default() },
        ShellFrame::Exit { code, timed_out, stopped, elapsed_ms } => {
            ShellEventView { kind: "exit".into(), code, timed_out, stopped, elapsed_ms, ..Default::default() }
        }
        ShellFrame::Denied { message } => ShellEventView { kind: "denied".into(), text: message, ..Default::default() },
    }
}

#[derive(Debug, Clone)]
pub struct NotificationView {
    pub at_ms: i64,
    /// `info`, `ok`, `ask` or `error`.
    pub level: String,
    pub text: String,
}

fn project_view(p: &Project) -> ProjectView {
    ProjectView { name: p.name.clone(), place: p.place.clone(), desktop_id: p.desktop.clone(), base: p.base.clone() }
}

fn diff_view(d: &FileDiff, accepted: &[bool]) -> FileDiffView {
    let (added, removed) = d.stats();
    FileDiffView {
        path: d.path.clone(),
        created: d.created,
        added: added as u32,
        removed: removed as u32,
        hunks: d
            .hunks
            .iter()
            .enumerate()
            .map(|(i, h)| HunkView {
                header: h.header(),
                accepted: accepted.get(i).copied().unwrap_or(true),
                lines: h
                    .lines
                    .iter()
                    .map(|l| DiffLineView {
                        tag: match l.tag {
                            LineTag::Context => "context",
                            LineTag::Added => "added",
                            LineTag::Removed => "removed",
                        }
                        .into(),
                        text: l.text.clone(),
                        old_no: l.old_no.map(|n| n as u32),
                        new_no: l.new_no.map(|n| n as u32),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn review_of(s: &CodeSession, note: Option<String>) -> ReviewView {
    ReviewView { explanation: s.explanation.clone(), files: s.diffs.iter().zip(&s.accepted).map(|(d, a)| diff_view(d, a)).collect(), note }
}

fn context_of(s: &CodeSession) -> ContextView {
    ContextView {
        targets: s.targets.clone(),
        files: s.files.iter().map(|f| ContextFileView { path: f.path.clone(), bytes: f.text.len() as u64 }).collect(),
        skipped: s.skipped.iter().map(|(p, why)| format!("{p}: {why}")).collect(),
        total_bytes: s.context_bytes() as u64,
    }
}

// ---- the desktop link --------------------------------------------------------------------------

/// The desktops this device is paired with.
#[frb(sync)]
pub fn link_desktops() -> Vec<DesktopView> {
    running_parts().map(|(_, _, l)| l.desktops().into_iter().map(desktop_view).collect()).unwrap_or_default()
}

/// Pairs with the desktop showing `code` (the QR code's text, or the code typed from its
/// screen). `label` is what the desktop lists this device as.
pub async fn link_pair(code: String, label: String) -> Result<DesktopView, String> {
    let (_, _, link) = running_parts()?;
    on_core(async move { link.pair(&code, &label).await.map(desktop_view).map_err(err) }).await
}

/// Whether `text` looks like a desktop pairing code (a scanned QR code or a typed one).
#[frb(sync)]
pub fn link_is_pair_code(text: String) -> bool {
    let t = text.trim();
    bragi_core::link::PairLink::parse(t).is_ok() || bragi_core::link::ShortCode::parse(t).is_ok()
}

/// Unpairs a desktop here and on the desktop (when it can be reached).
pub async fn link_forget(id: String) -> Result<(), String> {
    let (_, _, link) = running_parts()?;
    on_core(async move { link.forget(&id).await.map_err(err) }).await
}

/// Asks the desktop what it shares now (and refreshes its addresses).
pub async fn link_refresh(id: String) -> Result<DesktopView, String> {
    let (_, _, link) = running_parts()?;
    on_core(async move {
        link.hello(&id).await.map_err(err)?;
        link.desktops().into_iter().find(|d| d.id == id).map(desktop_view).ok_or_else(|| "that desktop isn't paired".to_string())
    })
    .await
}

/// `auto` (directly, then through Weirdnet), `direct` or `weirdnet` (the relay first).
#[frb(sync)]
pub fn link_route() -> String {
    running_parts().map(|(_, _, l)| l.route().as_str().to_string()).unwrap_or_else(|_| "auto".into())
}

#[frb(sync)]
pub fn link_set_route(route: String) -> Result<(), String> {
    let r = Route::parse(&route).ok_or_else(|| format!("unknown route {route}"))?;
    running_parts()?.2.set_route(r).map_err(err)
}

// ---- the project -------------------------------------------------------------------------------

/// Whether this device can work on a local folder (a computer; a phone uses a desktop).
#[frb(sync)]
pub fn code_local_available() -> bool {
    !cfg!(any(target_os = "android", target_os = "ios"))
}

/// Opens `path` on this device as the project (made, with starter files, when missing or empty).
pub async fn code_open_local(path: String) -> Result<OpenedView, String> {
    if !code_local_available() {
        return Err("this device has no project folders; pair with a desktop".into());
    }
    let (_, dir, _) = running_parts()?;
    on_core(async move {
        let (p, note) = Project::local(Path::new(&path), &dir.join("code-journal")).map_err(err)?;
        let view = project_view(&p);
        *session().lock().await = Some(CodeSession::new(p));
        Ok(OpenedView { project: view, note })
    })
    .await
}

/// Opens folder `sub` (empty: the root itself) of shared root `root` on desktop `id`.
pub async fn code_open_desktop(id: String, root: u32, sub: String) -> Result<OpenedView, String> {
    let (_, _, link) = running_parts()?;
    on_core(async move {
        let (p, note) = Project::desktop(link, &id, root as usize, &sub).await.map_err(err)?;
        let view = project_view(&p);
        *session().lock().await = Some(CodeSession::new(p));
        Ok(OpenedView { project: view, note })
    })
    .await
}

/// The open project, if any.
pub async fn code_project() -> Option<ProjectView> {
    session().lock().await.as_ref().map(|s| project_view(&s.project))
}

/// Closes the project (code mode stays, with no project).
pub async fn code_close() {
    *session().lock().await = None;
}

/// A folder's entries (`rel` relative to the root; empty: the root).
pub async fn code_list(rel: String) -> Result<Vec<EntryView>, String> {
    on_core(async move {
        let g = session().lock().await;
        let s = g.as_ref().ok_or("no project is open")?;
        let prefix = rel.trim_matches('/').to_string();
        let entries = s.project.list(&prefix).await.map_err(err)?;
        Ok(entries
            .into_iter()
            .map(|e| EntryView {
                path: if prefix.is_empty() { e.name.clone() } else { format!("{prefix}/{}", e.name) },
                name: e.name,
                dir: e.dir,
                size: e.size,
            })
            .collect())
    })
    .await
}

/// Sets the code context to `targets` (files and folders, relative to the root) and reads it.
pub async fn code_set_context(targets: Vec<String>) -> Result<ContextView, String> {
    on_core(async move {
        let mut g = session().lock().await;
        let s = g.as_mut().ok_or("no project is open")?;
        s.set_context(targets).await.map_err(err)?;
        Ok(context_of(s))
    })
    .await
}

pub async fn code_context() -> Option<ContextView> {
    session().lock().await.as_ref().map(context_of)
}

/// Asks the model for changes: the prompt goes with the context and the conversation so far
/// (a plan made in chat mode, earlier code turns). Stream it with `follow_chat`, then hand the
/// answer to `code_answered`.
pub async fn code_ask(conversation: Option<String>, prompt: String, model: Option<String>) -> Result<CodeAskView, String> {
    let (core, _, _) = running_parts()?;
    on_core(async move {
        let mut g = session().lock().await;
        let s = g.as_mut().ok_or("no project is open")?;
        let msgs = match &conversation {
            Some(id) => core.conversation(id).map(|c| c.messages).unwrap_or_default(),
            None => Vec::new(),
        };
        let state = core.state();
        let model_id = model.clone().or_else(|| state.catalog.default_model().map(str::to_string)).unwrap_or_default();
        let listed = state.catalog.context_for(&model_id);
        let (messages, note) = s.prepare(&prompt, &msgs, &model_id, listed).map_err(err)?;
        let h = core.complete(messages, model).map_err(err)?;
        Ok(CodeAskView { started: register(h), note })
    })
    .await
}

/// The finished answer to a `code_ask`: saves the ask and the answer as a code turn of
/// `conversation` (a new one when none) and returns the changes to review.
pub async fn code_answered(
    conversation: Option<String>,
    prompt: String,
    reply: String,
    reasoning: String,
    model: Option<String>,
) -> Result<AnsweredView, String> {
    let (core, _, _) = running_parts()?;
    on_core(async move {
        let ask = ChatMessage::new(Role::User, prompt, Mode::Code);
        let mut answer = ChatMessage::new(Role::Assistant, reply.clone(), Mode::Code);
        answer.reasoning = reasoning;
        answer.model = model;
        let id = core.record(conversation.as_deref(), vec![ask, answer]).map_err(err)?;
        let mut g = session().lock().await;
        let s = g.as_mut().ok_or("no project is open")?;
        let note = match s.answered(&reply).await {
            Ok([]) => Some("The answer proposes no change.".to_string()),
            Ok(_) => None,
            Err(e) => {
                s.discard();
                Some(format!("Couldn't read changes from the answer: {}", e.message))
            }
        };
        Ok(AnsweredView { conversation_id: id, review: review_of(s, note) })
    })
    .await
}

pub async fn code_review() -> Option<ReviewView> {
    session().lock().await.as_ref().map(|s| review_of(s, None))
}

/// Accepts or rejects one hunk.
pub async fn code_set_hunk(file: u32, hunk: u32, accept: bool) -> Option<ReviewView> {
    let mut g = session().lock().await;
    let s = g.as_mut()?;
    s.set_hunk(file as usize, hunk as usize, accept);
    Some(review_of(s, None))
}

/// Writes the accepted hunks (undo with `code_undo`).
pub async fn code_apply() -> Result<AppliedView, String> {
    on_core(async move {
        let mut g = session().lock().await;
        let s = g.as_mut().ok_or("no project is open")?;
        s.apply().await.map(|a| AppliedView { files: a.files }).map_err(err)
    })
    .await
}

pub async fn code_discard() {
    if let Some(s) = session().lock().await.as_mut() {
        s.discard();
    }
}

/// Puts back the last change written.
pub async fn code_undo() -> Result<AppliedView, String> {
    on_core(async move {
        let mut g = session().lock().await;
        let s = g.as_mut().ok_or("no project is open")?;
        s.undo().await.map(|a| AppliedView { files: a.files }).map_err(err)
    })
    .await
}

/// Runs `command` in the project folder (on the desktop for a desktop project), streaming its
/// output into `sink`; `id` (the app's own) is for `code_shell_stop`. Only the user starts these.
pub async fn code_shell(id: String, command: String, sink: StreamSink<ShellEventView>) -> Result<(), String> {
    let stop = Arc::new(AtomicBool::new(false));
    lock(shells()).insert(id.clone(), stop.clone());
    let key = id.clone();
    let (core, _, _) = running_parts()?;
    core.handle().spawn(async move {
        let project = session().lock().await.as_ref().map(|s| s.project.clone());
        let Some(project) = project else {
            let _ = sink.add(ShellEventView { kind: "exit".into(), text: "no project is open".into(), ..Default::default() });
            return;
        };
        let out = sink.clone();
        let r = project
            .shell(&command, stop, move |f| {
                let _ = out.add(shell_view(f));
            })
            .await;
        match r {
            // A stop on a desktop project ends without the desktop's exit frame.
            Ok(end @ ShellFrame::Exit { stopped: true, .. }) => {
                let _ = sink.add(shell_view(end));
            }
            Ok(_) => {}
            Err(e) => {
                let _ = sink.add(ShellEventView { kind: "err".into(), text: format!("{}\n", e.message), ..Default::default() });
                let _ = sink.add(ShellEventView { kind: "exit".into(), ..Default::default() });
            }
        }
        lock(shells()).remove(&key);
    });
    Ok(())
}

/// Stops a running command.
#[frb(sync)]
pub fn code_shell_stop(id: String) -> bool {
    lock(shells()).get(&id).map(|s| s.store(true, Ordering::Relaxed)).is_some()
}

/// Saves a shell run (`$ cmd`, its output and status) as a code turn of `conversation` (a new one
/// when none), so chat sees it too. Returns the conversation's id.
pub fn code_record_shell(conversation: Option<String>, transcript: String) -> Result<String, String> {
    let (core, _, _) = running_parts()?;
    core.record(conversation.as_deref(), vec![ChatMessage::new(Role::User, transcript, Mode::Code)]).map_err(err)
}

// ---- prompt history and notifications ---------------------------------------------------------

fn histories() -> &'static Mutex<HashMap<String, PromptHistory>> {
    static H: OnceLock<Mutex<HashMap<String, PromptHistory>>> = OnceLock::new();
    H.get_or_init(Default::default)
}

fn with_history<T>(key: &str, f: impl FnOnce(&mut PromptHistory) -> T) -> Result<T, String> {
    let (_, dir, _) = running_parts()?;
    let mut all = lock(histories());
    let h = all.entry(key.to_string()).or_insert_with(|| {
        let project = (key != "chat").then(|| PathBuf::from(key));
        PromptHistory::open(file_for(&dir, project.as_deref()))
    });
    Ok(f(h))
}

/// Up in the message box: the previous prompt sent in `key` (`chat`, or a project's place),
/// keeping `current` as the draft. None at the oldest.
#[frb(sync)]
pub fn prompt_history_up(key: String, current: String) -> Option<String> {
    with_history(&key, |h| h.up(&current)).ok().flatten()
}

/// Down: the next newer prompt, then the draft back.
#[frb(sync)]
pub fn prompt_history_down(key: String) -> Option<String> {
    with_history(&key, |h| h.down()).ok().flatten()
}

/// A prompt was sent: remember it (and stop browsing).
#[frb(sync)]
pub fn prompt_history_push(key: String, prompt: String) {
    let _ = with_history(&key, |h| h.push(&prompt));
}

fn log() -> &'static Mutex<Option<NotificationLog>> {
    static L: OnceLock<Mutex<Option<NotificationLog>>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(None))
}

fn with_log<T>(f: impl FnOnce(&mut NotificationLog) -> T) -> Result<T, String> {
    let (_, dir, _) = running_parts()?;
    let mut g = lock(log());
    let l = g.get_or_insert_with(|| NotificationLog::open(&dir));
    Ok(f(l))
}

/// Keeps a notification the app showed (`level`: `info`, `ok`, `ask` or `error`).
#[frb(sync)]
pub fn notify(level: String, text: String) {
    let level = match level.as_str() {
        "ok" => Level::Ok,
        "ask" => Level::Ask,
        "error" => Level::Error,
        _ => Level::Info,
    };
    let _ = with_log(|l| l.push(level, &text));
}

/// Every kept notification, newest first.
#[frb(sync)]
pub fn notifications() -> Vec<NotificationView> {
    with_log(|l| {
        l.entries()
            .iter()
            .rev()
            .map(|n| NotificationView {
                at_ms: n.at_ms,
                level: match n.level {
                    Level::Info => "info",
                    Level::Ok => "ok",
                    Level::Ask => "ask",
                    Level::Error => "error",
                }
                .into(),
                text: n.text.clone(),
            })
            .collect()
    })
    .unwrap_or_default()
}

#[frb(sync)]
pub fn notifications_unread() -> u32 {
    with_log(|l| l.unread() as u32).unwrap_or(0)
}

/// The list was opened: everything in it is read.
#[frb(sync)]
pub fn notifications_seen() {
    let _ = with_log(|l| l.seen());
}
