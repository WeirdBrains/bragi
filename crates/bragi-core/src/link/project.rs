//! Code mode on a project that is either on this device or on a paired desktop: the same calls
//! either way, so a shell (the Flutter app) draws one code mode. [`CodeSession`] adds the code
//! flow on top: the context, the prompt with the conversation, the answer's diffs and applying
//! the accepted hunks.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::ChatMessage;
use crate::code::budget::{TokenBudget, fit};
use crate::code::prompt::{parse_edits, proposals_with};
use crate::code::{CodeRequest, CodeTask, ContextFile, FileDiff, Journal, PromptMessage};
use crate::error::{CoreError, ErrorKind, Result};

use super::client::LinkClient;
use super::confine::Root;
use super::proto::{AppliedReply, Entry, GatherReply, ShellFrame, ShellRequest, WireChange};
use super::workspace;

enum Backend {
    Local { root: Root, journal: Journal },
    Desktop { client: Arc<LinkClient>, desktop: String, root: usize },
}

/// Where code mode works: a folder on this device, or a shared folder on a paired desktop.
/// Paths are relative to the root (`base` is the project folder inside it, where listing
/// starts and commands run).
pub struct Project {
    backend: Backend,
    /// The project folder, relative to the root (empty: the root itself).
    pub base: String,
    /// What to show: `myapp` and `~/src/myapp` or `myapp on Bragi terminal on desk`.
    pub name: String,
    pub place: String,
    /// The paired desktop's id, when it is on one.
    pub desktop: Option<String>,
}

impl Project {
    /// A folder on this device (made, with starter files, when missing or empty). The undo
    /// journal goes under `journal_dir`. Returns the project and what was set up.
    pub fn local(dir: &Path, journal_dir: &Path) -> Result<(Self, String)> {
        let started = crate::project::ensure(dir)?;
        let root = Root::new(dir)?;
        let journal = workspace::journal_for(journal_dir, &root);
        let name = root.path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let place = root.path().display().to_string();
        let note = started.note(root.path());
        Ok((Project { backend: Backend::Local { root, journal }, base: String::new(), name, place, desktop: None }, note))
    }

    /// Folder `sub` of shared root `root` on a paired desktop (made there when missing).
    pub async fn desktop(client: Arc<LinkClient>, desktop: &str, root: usize, sub: &str) -> Result<(Self, String)> {
        let hello = client.hello(desktop).await?;
        let info = hello.roots.get(root).cloned().ok_or_else(|| CoreError::new(ErrorKind::NotFound, "that project isn't shared any more"))?;
        let made = client.project(desktop, root, sub).await?;
        let base = if made.path == "." { String::new() } else { made.path };
        let name = if base.is_empty() { info.name.clone() } else { base.rsplit('/').next().unwrap_or(&base).to_string() };
        let place = format!("{}{} on {}", info.path, if base.is_empty() { String::new() } else { format!("/{base}") }, hello.label);
        Ok((
            Project {
                backend: Backend::Desktop { client, desktop: desktop.to_string(), root },
                base,
                name,
                place,
                desktop: Some(desktop.to_string()),
            },
            made.note,
        ))
    }

    /// The local folder, for a project on this device.
    pub fn local_path(&self) -> Option<PathBuf> {
        match &self.backend {
            Backend::Local { root, .. } => Some(root.path().to_path_buf()),
            Backend::Desktop { .. } => None,
        }
    }

    pub async fn list(&self, rel: &str) -> Result<Vec<Entry>> {
        match &self.backend {
            Backend::Local { root, .. } => workspace::list(root, rel),
            Backend::Desktop { client, desktop, root } => client.list(desktop, *root, rel).await,
        }
    }

    pub async fn read(&self, rel: &str) -> Result<Option<String>> {
        match &self.backend {
            Backend::Local { root, .. } => workspace::read(root, rel),
            Backend::Desktop { client, desktop, root } => client.read(desktop, *root, rel).await,
        }
    }

    pub async fn gather(&self, targets: &[String]) -> Result<GatherReply> {
        match &self.backend {
            Backend::Local { root, .. } => workspace::gather(root, targets),
            Backend::Desktop { client, desktop, root } => client.gather(desktop, *root, targets).await,
        }
    }

    pub async fn apply(&self, changes: Vec<WireChange>) -> Result<AppliedReply> {
        match &self.backend {
            Backend::Local { root, journal } => workspace::apply(root, journal, &changes),
            Backend::Desktop { client, desktop, root } => client.apply(desktop, *root, changes).await,
        }
    }

    pub async fn undo(&self, force: bool) -> Result<AppliedReply> {
        match &self.backend {
            Backend::Local { root, journal } => workspace::undo(root, journal, force),
            Backend::Desktop { client, desktop, root } => client.undo(desktop, *root, force).await,
        }
    }

    /// Runs `command` in the project folder, handing frames to `on`; `stop` ends it. Returns
    /// the last frame ([`ShellFrame::Exit`] or [`ShellFrame::Denied`]).
    pub async fn shell(&self, command: &str, stop: Arc<AtomicBool>, mut on: impl FnMut(ShellFrame) + Send + 'static) -> Result<ShellFrame> {
        match &self.backend {
            Backend::Local { root, .. } => {
                let cwd = root.resolve(&self.base)?;
                let command = command.to_string();
                tokio::task::spawn_blocking(move || {
                    let flag = stop.clone();
                    let e = crate::shell::run_streaming(
                        &command,
                        &cwd,
                        crate::shell::DEFAULT_TIMEOUT,
                        &move || flag.load(Ordering::Relaxed),
                        &mut |err, text| on(if err { ShellFrame::Err { text } } else { ShellFrame::Out { text } }),
                    )?;
                    let end = ShellFrame::Exit { code: e.code, timed_out: e.timed_out, stopped: e.stopped, elapsed_ms: e.elapsed_ms };
                    on(end.clone());
                    Ok(end)
                })
                .await
                .map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?
            }
            Backend::Desktop { client, desktop, root } => {
                let req = ShellRequest { root: *root, cwd: self.base.clone(), command: command.to_string(), timeout_secs: None };
                let started = std::time::Instant::now();
                let run = client.shell(desktop, req, &mut on);
                let watch = async {
                    while !stop.load(Ordering::Relaxed) {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                };
                tokio::select! {
                    r = run => r,
                    // Dropping the call stops the command on the desktop.
                    _ = watch => {
                        let end = ShellFrame::Exit { code: None, timed_out: false, stopped: true, elapsed_ms: started.elapsed().as_millis() as u64 };
                        Ok(end)
                    }
                }
            }
        }
    }
}

/// The code flow on a [`Project`].
pub struct CodeSession {
    /// Shared, so a long shell run doesn't hold the session.
    pub project: Arc<Project>,
    /// What the user added as context (files and folders, relative to the root).
    pub targets: Vec<String>,
    pub files: Vec<ContextFile>,
    pub skipped: Vec<(String, String)>,
    asked: Option<CodeRequest>,
    pub diffs: Vec<FileDiff>,
    pub accepted: Vec<Vec<bool>>,
    /// The answer's prose (what each change does), shown above the diffs.
    pub explanation: String,
}

impl CodeSession {
    pub fn new(project: Project) -> Self {
        CodeSession {
            project: Arc::new(project),
            targets: Vec::new(),
            files: Vec::new(),
            skipped: Vec::new(),
            asked: None,
            diffs: Vec::new(),
            accepted: Vec::new(),
            explanation: String::new(),
        }
    }

    /// Replaces the context with `targets` and reads them.
    pub async fn set_context(&mut self, targets: Vec<String>) -> Result<()> {
        let g = if targets.is_empty() { GatherReply { files: vec![], skipped: vec![] } } else { self.project.gather(&targets).await? };
        self.targets = targets;
        self.files = g.files;
        self.skipped = g.skipped;
        Ok(())
    }

    pub fn context_bytes(&self) -> usize {
        self.files.iter().map(|f| f.text.len()).sum()
    }

    /// The prompt for `prompt` with the context and the conversation, fitted to `listed` (the
    /// model's window as the catalog lists it). Returns the messages for
    /// [`Core::complete`](crate::Core::complete) and a note about files left out.
    pub fn prepare(&mut self, prompt: &str, conversation: &[ChatMessage], model: &str, listed: Option<u32>) -> Result<(Vec<PromptMessage>, String)> {
        let req = CodeRequest { task: CodeTask::Suggest, instruction: prompt.trim().to_string(), files: self.files.clone() };
        if req.instruction.is_empty() && req.files.is_empty() {
            return Err(CoreError::new(ErrorKind::BadRequest, "say what to do, or add files first"));
        }
        let budget = listed.map(TokenBudget::for_context);
        let max_conv = budget.map(|b| b.prompt_tokens() / 3).unwrap_or(4096);
        let (conv, used) = crate::code::turns::conversation_turns(conversation, max_conv);
        let (send, note) = match budget {
            Some(b) => {
                let b = TokenBudget { context: b.context.saturating_sub(b.scaled(used) as u32), ..b };
                let f = fit(&req, b, model)?;
                let note = f.note(model);
                (f.request, note)
            }
            None => (req, String::new()),
        };
        let messages = crate::code::turns::messages(&send, &conv)?;
        self.asked = Some(send);
        Ok((messages, note))
    }

    /// The finished answer: its diffs (every hunk accepted to begin with). Files the context
    /// didn't hold are read from the project.
    pub async fn answered(&mut self, reply: &str) -> Result<&[FileDiff]> {
        let req = self.asked.clone().ok_or_else(|| CoreError::new(ErrorKind::BadRequest, "no code request is waiting"))?;
        self.explanation = crate::code::prompt::explanation(reply);
        let edits = parse_edits(reply, &req.task)?;
        let mut originals: BTreeMap<String, Option<String>> = BTreeMap::new();
        for e in &edits {
            crate::code::prompt::safe_path(&e.path)?;
            if !req.files.iter().any(|f| f.path == e.path) && !originals.contains_key(&e.path) {
                originals.insert(e.path.clone(), self.project.read(&e.path).await?);
            }
        }
        let diffs = proposals_with(&req.files, &edits, &mut |p| Ok(originals.get(p).cloned().flatten()))?;
        self.accepted = diffs.iter().map(|d| vec![true; d.hunks.len()]).collect();
        self.diffs = diffs;
        Ok(&self.diffs)
    }

    pub fn set_hunk(&mut self, file: usize, hunk: usize, accept: bool) {
        if let Some(a) = self.accepted.get_mut(file).and_then(|f| f.get_mut(hunk)) {
            *a = accept;
        }
    }

    /// Writes the accepted hunks and reads the context again.
    pub async fn apply(&mut self) -> Result<AppliedReply> {
        let changes: Vec<WireChange> = self
            .diffs
            .iter()
            .zip(&self.accepted)
            .filter(|(_, a)| a.iter().any(|x| *x))
            .map(|(d, a)| WireChange { path: d.path.clone(), before: if d.created { None } else { Some(d.old.clone()) }, after: d.apply(a) })
            .collect();
        if changes.is_empty() {
            return Err(CoreError::new(ErrorKind::BadRequest, "no change is accepted"));
        }
        let a = self.project.apply(changes).await?;
        self.discard();
        let targets = self.targets.clone();
        let _ = self.set_context(targets).await;
        Ok(a)
    }

    pub fn discard(&mut self) {
        self.diffs.clear();
        self.accepted.clear();
        self.explanation.clear();
    }

    pub async fn undo(&mut self) -> Result<AppliedReply> {
        let a = self.project.undo(false).await?;
        let targets = self.targets.clone();
        let _ = self.set_context(targets).await;
        Ok(a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_local_session_asks_reviews_and_applies() {
        let d = tempfile::tempdir().unwrap();
        let (p, note) = Project::local(&d.path().join("app"), &d.path().join("journal")).unwrap();
        assert!(note.contains("README.md"), "{note}");
        let mut s = CodeSession::new(p);
        s.set_context(vec!["README.md".into()]).await.unwrap();
        assert_eq!(s.files.len(), 1);
        let (m, _) = s.prepare("add a usage line", &[], "m", Some(32_768)).unwrap();
        assert!(m.last().unwrap().content.contains("add a usage line"));
        let reply = "Adds usage.\n```bragi-edit path=README.md after=1\nUsage: run it.\n```\n```bragi-edit path=src/main.rs\nfn main() {}\n```\n";
        let diffs = s.answered(reply).await.unwrap();
        assert_eq!(diffs.len(), 2);
        assert!(diffs[1].created);
        s.set_hunk(1, 0, false);
        let a = s.apply().await.unwrap();
        assert_eq!(a.files, ["README.md"]);
        let readme = std::fs::read_to_string(d.path().join("app/README.md")).unwrap();
        assert!(readme.lines().nth(1) == Some("Usage: run it."), "{readme}");
        assert!(!d.path().join("app/src/main.rs").exists());
        assert!(s.files[0].text.contains("Usage"));
        s.undo().await.unwrap();
        assert!(!std::fs::read_to_string(d.path().join("app/README.md")).unwrap().contains("Usage"));
        let stop = Arc::new(AtomicBool::new(false));
        let out = Arc::new(std::sync::Mutex::new(String::new()));
        let o = out.clone();
        let end = s
            .project
            .shell("ls", stop, move |f| {
                if let ShellFrame::Out { text } = f {
                    o.lock().unwrap().push_str(&text);
                }
            })
            .await
            .unwrap();
        assert!(matches!(end, ShellFrame::Exit { code: Some(0), .. }));
        assert!(out.lock().unwrap().contains("README.md"));
    }
}
