//! Code mode's state: the context, the request, the answer and the review of its diffs. All
//! the prompt, parsing, diff and undo logic is `bragi_core::code`, shared with the editor
//! extension; this only keeps track of what the user picked.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bragi_core::code::budget::{Overflow, TokenBudget, fit, request_tokens, thousands};
use bragi_core::code::prompt::{self, explanation, parse_edits, proposals};
use bragi_core::code::{CodeRequest, CodeTask, FileChange, FileDiff, Gathered, Journal, Limits, gather, parse_target};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Picking context and typing a request.
    Compose,
    /// The answer is streaming.
    Asking,
    /// Accepting or rejecting hunks.
    Review,
}

pub struct CodeState {
    /// Paths in requests and diffs are relative to this folder.
    pub root: PathBuf,
    /// What the user added, as typed.
    pub context: Vec<PathBuf>,
    pub gathered: Gathered,
    pub phase: Phase,
    pub request: Option<CodeRequest>,
    pub answer: String,
    pub explanation: String,
    pub diffs: Vec<FileDiff>,
    /// Per file, per hunk.
    pub accepted: Vec<Vec<bool>>,
    pub file: usize,
    pub hunk: usize,
    pub side_by_side: bool,
    pub journal: Journal,
    /// The request as asked (before fitting), for one retry when a server says it's too big.
    pub asked: Option<Asked>,
    /// The conversation sent along with the request that is out (kept for a retry).
    pub turns: Vec<bragi_core::code::PromptMessage>,
    /// The line typed for the request that is out (saved in the conversation with its answer).
    pub asked_line: String,
    /// Prompts typed in code mode, for Up and Down (this project's history).
    pub history: bragi_core::prompt_history::PromptHistory,
    /// Per model: the context window a server reported, and how far its token count ran over
    /// the estimate. Wins over the catalog, which only says what the node was configured with.
    pub learned: HashMap<String, Learned>,
}

/// A request that is out, as the user asked it.
#[derive(Debug, Clone, PartialEq)]
pub struct Asked {
    pub request: CodeRequest,
    pub model: String,
    /// The window it was fitted to, if one was known.
    pub budget: Option<TokenBudget>,
    /// Whether this is already the retry.
    pub retried: bool,
    /// Tokens the conversation sent along takes.
    pub reserve: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Learned {
    pub context: Option<u32>,
    pub ratio: f64,
}

/// What to do after a code request failed.
#[derive(Debug, Clone, PartialEq)]
pub enum AfterError {
    /// Ask again with this (smaller) request; the string says why.
    Retry(CodeRequest, String),
    /// Show this and stop.
    Fail(String),
}

/// What a typed line in code mode asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Add(Vec<String>),
    Drop(String),
    Clear,
    Root(String),
    Pick,
    Ask(CodeRequest),
    Undo {
        force: bool,
    },
    /// `!cmd` or `/shell cmd`: run it in the root.
    Shell(String),
    /// `/notifications`: the notification list.
    Notifications,
    /// `/chat`, `/code` or `/mode` (the other one): switch modes in the same conversation.
    Mode(Option<bragi_core::Mode>),
    Nothing,
}

impl CodeState {
    pub fn new(root: PathBuf, journal_dir: PathBuf) -> Self {
        Self {
            root,
            context: Vec::new(),
            gathered: Gathered::default(),
            phase: Phase::Compose,
            request: None,
            answer: String::new(),
            explanation: String::new(),
            diffs: Vec::new(),
            accepted: Vec::new(),
            file: 0,
            hunk: 0,
            side_by_side: false,
            journal: Journal::new(journal_dir),
            asked: None,
            learned: HashMap::new(),
            asked_line: String::new(),
            turns: Vec::new(),
            history: bragi_core::prompt_history::PromptHistory::in_memory(),
        }
    }

    /// Re-reads the context from disk.
    pub fn refresh(&mut self) -> Result<(), String> {
        self.gathered = gather(&self.root, &self.context, Limits::default()).map_err(|e| e.message)?;
        Ok(())
    }

    pub fn add(&mut self, paths: &[String]) -> Result<String, String> {
        for p in paths {
            let p = expand(p);
            let full = if p.is_absolute() { p.clone() } else { self.root.join(&p) };
            if !full.exists() {
                return Err(format!("{} doesn't exist", p.display()));
            }
            if !self.context.contains(&p) {
                self.context.push(p);
            }
        }
        self.refresh()?;
        Ok(format!("Context: {} files, {} KiB{}", self.gathered.files.len(), self.gathered.total_bytes().div_ceil(1024), self.skipped_note()))
    }

    /// Whether `rel` (a path under the root, `/`-separated) is in the context, by itself or
    /// through a folder that was added.
    pub fn has_file(&self, rel: &str) -> bool {
        self.gathered.get(rel).is_some()
    }

    /// Adds or takes out one file: what the file picker's Space does. Taking out a file that
    /// came with a folder swaps the folder for its other files. Adding is refused past the
    /// limits (files, bytes per file, bytes in all).
    pub fn set_file(&mut self, rel: &str, on: bool) -> Result<(), String> {
        let limits = Limits::default();
        let rel_of = |root: &Path, p: &Path| bragi_core::code::context::relative(root, &if p.is_absolute() { p.to_path_buf() } else { root.join(p) });
        if on {
            if self.has_file(rel) {
                return Ok(());
            }
            let size = std::fs::metadata(self.root.join(rel)).map_err(|e| format!("{rel}: {e}"))?.len() as usize;
            if size > limits.max_file_bytes {
                return Err(format!("{rel} is {} KiB; one file may be at most {} KiB", size.div_ceil(1024), limits.max_file_bytes / 1024));
            }
            if self.gathered.files.len() >= limits.max_files {
                return Err(format!("The context holds {} files already; take one out first", limits.max_files));
            }
            if self.gathered.total_bytes() + size > limits.max_total_bytes {
                return Err(format!("{rel} would take the context past {} KiB; take something out first", limits.max_total_bytes / 1024));
            }
            self.context.push(PathBuf::from(rel));
            return self.refresh();
        }
        if let Some(i) = self.context.iter().position(|p| rel_of(&self.root, p) == rel) {
            self.context.remove(i);
            return self.refresh();
        }
        // It came with a folder: swap the folder for the files it brought, less this one.
        let mut kept = Vec::new();
        let mut found = false;
        for p in std::mem::take(&mut self.context) {
            let r = rel_of(&self.root, &p);
            let under = r == "." || r.is_empty() || rel.starts_with(&format!("{}/", r.trim_end_matches('/')));
            if under && self.root.join(&p).is_dir() {
                found = true;
                let prefix = if r == "." || r.is_empty() { String::new() } else { format!("{}/", r.trim_end_matches('/')) };
                kept.extend(self.gathered.files.iter().filter(|f| f.path.starts_with(&prefix) && f.path != rel).map(|f| PathBuf::from(&f.path)));
            } else {
                kept.push(p);
            }
        }
        kept.dedup();
        self.context = kept;
        if !found {
            return Ok(());
        }
        self.refresh()
    }

    /// `3 files, 12 KiB of 512 KiB`.
    pub fn summary(&self) -> String {
        let l = Limits::default();
        let n = self.gathered.files.len();
        format!("{n} file{}, {} KiB of {} KiB", plural(n), self.gathered.total_bytes().div_ceil(1024), l.max_total_bytes / 1024)
    }

    /// The context's estimated tokens against what `model` can take: `~95.7k tokens; inkling-2-mini
    /// takes ~23k (32.8k context)`. `context` is what the catalog lists for it.
    pub fn token_summary(&self, model: &str, context: Option<u32>) -> String {
        let n: usize = self.gathered.files.iter().map(bragi_core::code::budget::file_tokens).sum();
        match self.budget_for(model, context) {
            Some(b) => format!(
                "~{} tokens; {model} takes ~{} ({} context)",
                thousands(b.scaled(n)),
                thousands(b.prompt_tokens()),
                thousands(b.context as usize)
            ),
            None => format!("~{} tokens", thousands(n)),
        }
    }

    /// The budget for `model`: what a server reported beats what the catalog lists.
    pub fn budget_for(&self, model: &str, listed: Option<u32>) -> Option<TokenBudget> {
        let learned = self.learned.get(model);
        let context = learned.and_then(|l| l.context).or(listed)?;
        Some(TokenBudget::for_context(context).with_ratio(learned.map(|l| l.ratio).unwrap_or(1.0)))
    }

    /// Fits `req` into `model`'s window (`listed` is the catalog's figure) and remembers it for
    /// a retry. Returns what to send and a note about files left out (empty when none were).
    /// `reserve` is what the conversation sent along takes (estimated tokens).
    pub fn prepare(&mut self, req: CodeRequest, model: &str, listed: Option<u32>, reserve: usize) -> Result<(CodeRequest, String), String> {
        let budget = self.budget_for(model, listed).map(|b| less(b, reserve));
        let (send, note) = match budget {
            Some(b) => {
                let f = fit(&req, b, model).map_err(|e| e.message)?;
                let note = f.note(model);
                (f.request, note)
            }
            None => (req.clone(), String::new()),
        };
        self.asked = Some(Asked { request: req, model: model.to_string(), budget, retried: false, reserve });
        Ok((send, note))
    }

    /// After a failed request: when the server said the prompt was too big for its context,
    /// learn its window and ratio and fit the request again (once); otherwise a plain message.
    pub fn after_error(&mut self, message: &str) -> AfterError {
        let Some(o) = Overflow::parse(message) else { return AfterError::Fail(message.to_string()) };
        let Some(asked) = self.asked.clone() else { return AfterError::Fail(o.message("the model")) };
        let model = asked.model.clone();
        if asked.retried {
            return AfterError::Fail(o.message(&model));
        }
        // How far the server's count ran over the estimate of what was actually sent.
        let sent = self.request.as_ref().and_then(|r| request_tokens(r).ok()).unwrap_or(0);
        let old = self.learned.get(&model).copied().unwrap_or(Learned { context: None, ratio: 1.0 });
        let mut ratio = old.ratio;
        if let (Some(p), true) = (o.prompt_tokens, sent > 0) {
            ratio = ratio.max(p as f64 / sent as f64 * 1.05);
        } else if o.context.is_none() {
            // No numbers at all: be half again as strict.
            ratio *= 1.5;
        }
        let context = o.context.or(asked.budget.map(|b| b.context));
        let Some(context) = context else { return AfterError::Fail(o.message(&model)) };
        self.learned.insert(model.clone(), Learned { context: Some(context), ratio });
        let budget = less(TokenBudget::for_context(context).with_ratio(ratio), asked.reserve);
        match fit(&asked.request, budget, &model) {
            Ok(f) => {
                let total = asked.request.files.len();
                let kept = f.request.files.len();
                self.asked = Some(Asked { budget: Some(budget), retried: true, ..asked });
                let why = format!(
                    "{model} has a {} context; asking again with {kept} of {total} file{} (~{} tokens)",
                    thousands(context as usize),
                    plural(total),
                    thousands(f.tokens)
                );
                AfterError::Retry(f.request, why)
            }
            Err(e) => AfterError::Fail(e.message),
        }
    }

    pub fn skipped_note(&self) -> String {
        match self.gathered.skipped.len() {
            0 => String::new(),
            n => format!(" ({n} left out: ignored, binary, secret or too big)"),
        }
    }

    /// Reads a typed line: `/files`, `/add`, `/drop`, `/clear`, `/root`, `/rewrite FILE[:A-B] how`,
    /// `/suggest [what]`, `/undo[!]`; anything else asks for suggestions about it.
    pub fn parse(&mut self, line: &str) -> Result<Command, String> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(Command::Nothing);
        }
        if bragi_core::shell::is_shell_line(line) {
            return bragi_core::shell::parse(line)
                .map(|c| Command::Shell(c.to_string()))
                .ok_or_else(|| "usage: !COMMAND or /shell COMMAND".to_string());
        }
        let (cmd, rest) = match line.strip_prefix('/') {
            Some(c) => c.split_once(char::is_whitespace).map(|(a, b)| (a, b.trim())).unwrap_or((c, "")),
            None => ("suggest", line),
        };
        Ok(match cmd {
            "add" | "a" if !rest.is_empty() => Command::Add(rest.split_whitespace().map(str::to_string).collect()),
            "drop" | "d" if !rest.is_empty() => Command::Drop(rest.to_string()),
            "clear" => Command::Clear,
            "files" | "pick" | "f" => Command::Pick,
            "notifications" | "notices" => Command::Notifications,
            "chat" | "plan" | "code" | "mode" => return mode_command(line),
            "root" if !rest.is_empty() => Command::Root(rest.to_string()),
            "undo" => Command::Undo { force: false },
            "undo!" => Command::Undo { force: true },
            "rewrite" | "r" => {
                let (target, how) = rest.split_once(char::is_whitespace).ok_or("usage: /rewrite FILE[:FROM-TO] how to rewrite it")?;
                let target = parse_target(target).map_err(|e| e.message)?;
                let path = PathBuf::from(&target.path);
                if !self.context.contains(&path) {
                    self.add(std::slice::from_ref(&target.path))?;
                }
                self.refresh()?;
                let rel = bragi_core::code::context::relative(&self.root, &self.root.join(&path));
                let target = bragi_core::code::Target { path: rel, ..target };
                Command::Ask(CodeRequest { task: CodeTask::Rewrite(target), instruction: how.trim().to_string(), files: self.gathered.files.clone() })
            }
            "suggest" | "s" => {
                self.refresh()?;
                // No context: the model may still create files, so the ask has to say what.
                if self.gathered.files.is_empty() && rest.is_empty() {
                    return Err("add files or a folder first (/add src), or say what to create".into());
                }
                Command::Ask(CodeRequest { task: CodeTask::Suggest, instruction: rest.to_string(), files: self.gathered.files.clone() })
            }
            _ => return Err(format!("unknown command /{cmd}; F1 lists them")),
        })
    }

    /// Works in `dir` from now on: made (with starter files) when it doesn't exist or is
    /// empty, the context emptied and this project's prompt history loaded.
    pub fn set_root(&mut self, dir: PathBuf, home: &Path) -> Result<String, String> {
        let started = bragi_core::project::ensure(&dir).map_err(|e| e.message)?;
        let dir = dir.canonicalize().unwrap_or(dir);
        self.root = dir;
        self.context.clear();
        self.history = bragi_core::prompt_history::PromptHistory::open(bragi_core::prompt_history::file_for(home, Some(&self.root)));
        self.refresh()?;
        let note = started.note(&self.root);
        Ok(if note.is_empty() { format!("Working in {}", self.root.display()) } else { note })
    }

    pub fn drop_path(&mut self, what: &str) -> Result<String, String> {
        let before = self.context.len();
        match what.parse::<usize>() {
            Ok(n) if n >= 1 && n <= self.context.len() => {
                self.context.remove(n - 1);
            }
            _ => self.context.retain(|p| p != Path::new(what)),
        }
        if self.context.len() == before {
            return Err(format!("{what} isn't in the context"));
        }
        self.refresh()?;
        Ok(format!("Context: {} files", self.gathered.files.len()))
    }

    pub fn start(&mut self, req: CodeRequest) {
        self.request = Some(req);
        self.answer.clear();
        self.explanation.clear();
        self.diffs.clear();
        self.accepted.clear();
        self.phase = Phase::Asking;
    }

    /// The finished answer: its diffs, all hunks accepted to begin with. Returns a notice.
    pub fn answered(&mut self, reply: &str) -> String {
        self.answer = reply.to_string();
        self.explanation = explanation(reply);
        let Some(req) = &self.request else { return String::new() };
        let result = parse_edits(reply, &req.task).and_then(|edits| proposals(&self.root, &req.files, &edits));
        match result {
            Ok(diffs) if diffs.is_empty() => {
                self.phase = Phase::Compose;
                "The answer proposes no change.".into()
            }
            Ok(diffs) => {
                self.accepted = diffs.iter().map(|d| vec![true; d.hunks.len()]).collect();
                let hunks: usize = diffs.iter().map(|d| d.hunks.len()).sum();
                let n = diffs.len();
                self.diffs = diffs;
                self.file = 0;
                self.hunk = 0;
                self.phase = Phase::Review;
                format!("{hunks} change{} in {n} file{}: y accept · n reject · w write · Esc discard", plural(hunks), plural(n))
            }
            Err(e) => {
                self.phase = Phase::Compose;
                format!("Couldn't read changes from the answer: {}", e.message)
            }
        }
    }

    pub fn failed(&mut self) {
        self.phase = Phase::Compose;
    }

    pub fn current(&self) -> Option<(&FileDiff, usize)> {
        self.diffs.get(self.file).map(|d| (d, self.hunk))
    }

    /// Moves to the next (or previous) hunk, across files.
    pub fn step(&mut self, forward: bool) {
        let flat: Vec<(usize, usize)> = self.diffs.iter().enumerate().flat_map(|(f, d)| (0..d.hunks.len()).map(move |h| (f, h))).collect();
        let Some(i) = flat.iter().position(|p| *p == (self.file, self.hunk)) else { return };
        let j = if forward { (i + 1).min(flat.len() - 1) } else { i.saturating_sub(1) };
        (self.file, self.hunk) = flat[j];
    }

    pub fn set_current(&mut self, accept: bool) {
        if let Some(a) = self.accepted.get_mut(self.file).and_then(|f| f.get_mut(self.hunk)) {
            *a = accept;
        }
    }

    pub fn toggle_current(&mut self) {
        if let Some(a) = self.accepted.get_mut(self.file).and_then(|f| f.get_mut(self.hunk)) {
            *a = !*a;
        }
    }

    pub fn set_all(&mut self, accept: bool) {
        for f in &mut self.accepted {
            f.iter_mut().for_each(|a| *a = accept);
        }
    }

    pub fn counts(&self) -> (usize, usize) {
        let all: usize = self.accepted.iter().map(Vec::len).sum();
        (self.accepted.iter().flatten().filter(|a| **a).count(), all)
    }

    /// The files with their accepted hunks, as writes.
    pub fn changes(&self) -> Vec<FileChange> {
        changes(&self.root, &self.diffs, &self.accepted)
    }

    /// Writes the accepted hunks (all or nothing) and leaves review. Returns a notice.
    pub fn write(&mut self) -> Result<String, String> {
        let changes = self.changes();
        if changes.is_empty() {
            self.discard();
            return Ok("Nothing accepted; no file changed.".into());
        }
        let applied = self.journal.apply(&changes).map_err(|e| e.message)?;
        self.discard();
        let _ = self.refresh();
        let n = applied.files.len();
        Ok(format!("Wrote {n} file{}. /undo puts {} back.", plural(n), if n == 1 { "it" } else { "them" }))
    }

    pub fn discard(&mut self) {
        self.diffs.clear();
        self.accepted.clear();
        self.phase = Phase::Compose;
    }

    pub fn undo(&mut self, force: bool) -> Result<String, String> {
        let undone = self.journal.undo_last(force).map_err(|e| e.message)?;
        let _ = self.refresh();
        let names: Vec<String> = undone.files.iter().map(|p| bragi_core::code::context::relative(&self.root, p)).collect();
        Ok(format!("Undid the last change: {}", names.join(", ")))
    }

    /// The prompt messages for `req`, for `Core::complete`.
    /// The prompt: the code instructions, then the conversation so far (a plan made in chat
    /// mode, earlier code turns), then the files and the ask.
    pub fn messages(req: &CodeRequest, conversation: &[bragi_core::code::PromptMessage]) -> Result<Vec<bragi_core::code::PromptMessage>, String> {
        let mut m = prompt::build(req).map_err(|e| e.message)?;
        let at = m.iter().take_while(|x| x.role == bragi_core::code::PromptRole::System).count();
        m.splice(at..at, conversation.iter().cloned());
        Ok(m)
    }
}

/// `/chat`, `/plan`, `/code` or `/mode [chat|code]`; an error for any other line.
pub fn mode_command(line: &str) -> Result<Command, String> {
    let line = line.trim();
    let (cmd, rest) = line.split_once(char::is_whitespace).map(|(a, b)| (a, b.trim())).unwrap_or((line, ""));
    let mode = |w: &str| match w {
        "chat" | "plan" => Some(bragi_core::Mode::Chat),
        "code" => Some(bragi_core::Mode::Code),
        _ => None,
    };
    match cmd {
        "/chat" | "/plan" | "/code" if rest.is_empty() => Ok(Command::Mode(mode(&cmd[1..]))),
        "/mode" if rest.is_empty() => Ok(Command::Mode(None)),
        "/mode" => mode(rest).map(|m| Command::Mode(Some(m))).ok_or_else(|| "usage: /mode [chat|code]".into()),
        _ => Err(format!("{line} isn't a mode command")),
    }
}

/// `b` with `reserve` tokens of its window already taken.
fn less(b: TokenBudget, reserve: usize) -> TokenBudget {
    TokenBudget { context: b.context.saturating_sub(b.scaled(reserve) as u32), ..b }
}

/// The conversation as prompt turns, newest kept, within `max` estimated tokens: text only,
/// streaming and empty answers left out, turns from the same side joined. Returns the turns and
/// their estimate.
pub fn conversation_turns(messages: &[bragi_core::ChatMessage], max: usize) -> (Vec<bragi_core::code::PromptMessage>, usize) {
    use bragi_core::code::budget::{PER_MESSAGE, estimate_tokens};
    use bragi_core::code::{PromptMessage, PromptRole};
    let done: Vec<bragi_core::ChatMessage> =
        messages.iter().filter(|m| m.state != bragi_core::MessageState::Streaming && !m.text.trim().is_empty()).cloned().collect();
    let mut merged = bragi_core::merge_turns(&done);
    // The ask that follows is a user turn: the history has to end with an answer.
    while merged.last().is_some_and(|m| m.role == bragi_core::Role::User) {
        merged.pop();
    }
    let mut out = Vec::new();
    let mut used = 0;
    for m in merged.iter().rev() {
        let n = estimate_tokens(&m.text) + PER_MESSAGE;
        if used + n > max {
            break;
        }
        used += n;
        let role = if m.role == bragi_core::Role::User { PromptRole::User } else { PromptRole::Assistant };
        out.push(PromptMessage { role, content: m.text.clone() });
    }
    out.reverse();
    // And start with a question.
    while out.first().is_some_and(|m| m.role == PromptRole::Assistant) {
        let m = out.remove(0);
        used = used.saturating_sub(estimate_tokens(&m.content) + PER_MESSAGE);
    }
    (out, used)
}

/// `diffs` with their `accepted` hunks as file writes under `root` (files with none accepted are left out).
pub fn changes(root: &Path, diffs: &[FileDiff], accepted: &[Vec<bool>]) -> Vec<FileChange> {
    diffs
        .iter()
        .zip(accepted)
        .filter(|(_, a)| a.iter().any(|x| *x))
        .map(|(d, a)| FileChange { path: root.join(&d.path), before: if d.created { None } else { Some(d.old.clone()) }, after: d.apply(a) })
        .collect()
}

pub fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// `~/x` as a path under the home folder.
pub fn expand(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)).unwrap_or_else(|| PathBuf::from(p)),
        None => PathBuf::from(p),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> (tempfile::TempDir, CodeState) {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("src")).unwrap();
        let body: String = (1..=20).map(|i| format!("line {i}\n")).collect();
        std::fs::write(d.path().join("src/a.rs"), body).unwrap();
        let s = CodeState::new(d.path().to_path_buf(), d.path().join(".journal"));
        (d, s)
    }

    #[test]
    fn commands_parse() {
        let (_d, mut s) = project();
        assert_eq!(s.parse("/add src").unwrap(), Command::Add(vec!["src".into()]));
        assert!(matches!(s.parse("/undo!").unwrap(), Command::Undo { force: true }));
        assert!(s.parse("/suggest").is_err(), "a bare suggest needs context");
        // With no context the ask can still create files.
        assert!(matches!(s.parse("make a hello world").unwrap(), Command::Ask(CodeRequest { ref files, .. }) if files.is_empty()));
        assert_eq!(s.parse("!ls -la").unwrap(), Command::Shell("ls -la".into()));
        assert_eq!(s.parse("/shell git status").unwrap(), Command::Shell("git status".into()));
        assert!(s.parse("!").is_err());
        assert_eq!(s.parse("/notifications").unwrap(), Command::Notifications);
        assert_eq!(s.parse("/chat").unwrap(), Command::Mode(Some(bragi_core::Mode::Chat)));
        assert_eq!(s.parse("/mode").unwrap(), Command::Mode(None));
        assert!(s.parse("/mode sideways").is_err());
        s.add(&["src".into()]).unwrap();
        assert!(matches!(s.parse("tidy it").unwrap(), Command::Ask(CodeRequest { task: CodeTask::Suggest, .. })));
        let Command::Ask(r) = s.parse("/rewrite src/a.rs:2-3 shorter").unwrap() else { panic!() };
        assert_eq!(r.task, CodeTask::Rewrite(parse_target("src/a.rs:2-3").unwrap()));
        assert!(s.parse("/rewrite src/a.rs").is_err());
        assert!(s.parse("/frob").is_err());
    }

    #[test]
    fn review_write_and_undo() {
        let (d, mut s) = project();
        let Command::Ask(req) = s.parse("/rewrite src/a.rs:2-2 shout").unwrap() else { panic!() };
        s.start(req);
        let reply = "```bragi-edit path=src/a.rs lines=2-2\nLINE 2\n```\n\n```bragi-edit path=src/a.rs lines=19-19\nLINE 19\n```\nDone.";
        let note = s.answered(reply);
        assert_eq!(s.phase, Phase::Review, "{note}");
        assert_eq!(s.counts(), (2, 2));
        s.step(true);
        s.set_current(false);
        assert_eq!(s.counts(), (1, 2));
        s.write().unwrap();
        let text = std::fs::read_to_string(d.path().join("src/a.rs")).unwrap();
        assert!(text.contains("LINE 2\n") && text.contains("line 19\n"));
        s.undo(false).unwrap();
        assert!(std::fs::read_to_string(d.path().join("src/a.rs")).unwrap().contains("line 2\n"));
    }

    #[test]
    fn single_files_come_and_go_within_the_limits() {
        let (d, mut st) = project();
        let root = d.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        for n in ["a.rs", "b.rs", "c.rs"] {
            std::fs::write(root.join("src").join(n), "fn x() {}\n").unwrap();
        }
        st.set_file("src/a.rs", true).unwrap();
        st.set_file("src/a.rs", true).unwrap();
        assert!(st.has_file("src/a.rs") && st.gathered.files.len() == 1);
        // A folder, then one of its files out: the others stay.
        st.add(&["src".to_string()]).unwrap();
        assert!(st.has_file("src/b.rs") && st.has_file("src/c.rs"));
        st.set_file("src/b.rs", false).unwrap();
        assert!(!st.has_file("src/b.rs") && st.has_file("src/a.rs") && st.has_file("src/c.rs"));
        st.set_file("src/a.rs", false).unwrap();
        st.set_file("src/c.rs", false).unwrap();
        assert!(st.gathered.files.iter().all(|f| !f.path.starts_with("src/")));
        // Limits.
        std::fs::write(root.join("big.txt"), "x".repeat(300 * 1024)).unwrap();
        assert!(st.set_file("big.txt", true).unwrap_err().contains("at most 256 KiB"));
        for i in 0..2 {
            std::fs::write(root.join(format!("m{i}.txt")), "y".repeat(200 * 1024)).unwrap();
        }
        st.set_file("m0.txt", true).unwrap();
        st.set_file("m1.txt", true).unwrap();
        std::fs::write(root.join("m2.txt"), "y".repeat(200 * 1024)).unwrap();
        assert!(st.set_file("m2.txt", true).unwrap_err().contains("past 512 KiB"));
        assert!(!st.has_file("m2.txt"));
    }

    fn big_project(files: usize, bytes: usize) -> (tempfile::TempDir, CodeState) {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("ld4")).unwrap();
        let line = "    total = compute(values, weights)  # pad\n";
        for i in 0..files {
            std::fs::write(d.path().join(format!("ld4/m{i:02}.py")), line.repeat(bytes / line.len())).unwrap();
        }
        let mut s = CodeState::new(d.path().to_path_buf(), d.path().join(".journal"));
        s.add(&["ld4".into()]).unwrap();
        (d, s)
    }

    const LLAMA_OVERFLOW: &str = r#"bad request 400: {"error":{"code":400,"message":"request (91723 tokens) exceeds the available context size (32768 tokens), try increasing it","type":"exceed_context_size_error","n_prompt_tokens":91723,"n_ctx":32768}}"#;

    #[test]
    fn a_request_is_fitted_to_the_listed_context() {
        let (_d, mut s) = big_project(22, 10 * 1024);
        let Command::Ask(req) = s.parse("tidy it").unwrap() else { panic!() };
        let (sent, note) = s.prepare(req.clone(), "inkling-2-mini", Some(32_768), 0).unwrap();
        assert!(sent.files.len() < 22 && !sent.files.is_empty());
        assert!(note.contains("of 22 files") && note.contains("inkling-2-mini"), "{note}");
        assert!(request_tokens(&sent).unwrap() <= TokenBudget::for_context(32_768).prompt_tokens());
        assert!(s.token_summary("inkling-2-mini", Some(32_768)).contains("32.8k context"));
        // No window known: sent as is (a server refusal teaches it).
        let (sent, note) = s.prepare(req.clone(), "unknown", None, 0).unwrap();
        assert_eq!(sent, req);
        assert!(note.is_empty());
    }

    #[test]
    fn a_context_overflow_retries_once_with_what_the_server_said() {
        let (_d, mut s) = big_project(22, 10 * 1024);
        let Command::Ask(req) = s.parse("tidy it").unwrap() else { panic!() };
        // The catalog said nothing, so all 22 files went out and the server refused.
        let (sent, _) = s.prepare(req, "inkling-2-mini", None, 0).unwrap();
        assert_eq!(sent.files.len(), 22);
        s.start(sent);
        s.failed();
        let AfterError::Retry(again, why) = s.after_error(LLAMA_OVERFLOW) else { panic!("no retry") };
        assert!(again.files.len() < 22, "{why}");
        assert!(why.contains("32.8k context") && why.contains("of 22 files"), "{why}");
        assert_eq!(s.learned["inkling-2-mini"].context, Some(32_768));
        // The next request is fitted before it goes out.
        assert!(s.budget_for("inkling-2-mini", None).is_some());
        // A second refusal is a plain message, not raw JSON and not another retry.
        s.start(again);
        let AfterError::Fail(m) = s.after_error(LLAMA_OVERFLOW) else { panic!("retried twice") };
        assert!(m.contains("too big for inkling-2-mini") && !m.contains('{'), "{m}");
        // Other errors pass through.
        assert_eq!(s.after_error("no inference node answered"), AfterError::Fail("no inference node answered".into()));
    }

    #[test]
    fn a_new_root_is_made_with_starter_files_and_its_own_history() {
        let home = tempfile::tempdir().unwrap();
        let (d, mut s) = project();
        let fresh = d.path().join("brand-new");
        let note = s.set_root(fresh.clone(), home.path()).unwrap();
        assert!(note.contains("created the folder"), "{note}");
        assert!(fresh.join("README.md").exists() && fresh.join(".gitignore").exists());
        s.history.push("make it say hi");
        let again = bragi_core::prompt_history::PromptHistory::open(bragi_core::prompt_history::file_for(home.path(), Some(&fresh)));
        assert_eq!(again.entries(), ["make it say hi"]);
        // An existing project is opened as it is.
        let note = s.set_root(d.path().to_path_buf(), home.path()).unwrap();
        assert!(note.starts_with("Working in"), "{note}");
        assert!(!d.path().join("README.md").exists());
    }

    #[test]
    fn the_conversation_goes_along_newest_first_and_ends_on_an_answer() {
        use bragi_core::{ChatMessage, Mode, Role};
        let msgs = vec![
            ChatMessage::new(Role::User, "plan it", Mode::Chat),
            ChatMessage::new(Role::Assistant, "a plan", Mode::Chat),
            ChatMessage::new(Role::User, "make it", Mode::Code),
            ChatMessage::new(Role::User, "$ ls", Mode::Code),
            ChatMessage::new(Role::Assistant, "done", Mode::Code),
            ChatMessage::new(Role::User, "and a test?", Mode::Code),
        ];
        let (turns, used) = conversation_turns(&msgs, 10_000);
        let texts: Vec<&str> = turns.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(texts, ["plan it", "a plan", "make it\n\n$ ls", "done"]);
        assert!(used > 0);
        // A small budget keeps the newest whole turns only.
        let (turns, _) = conversation_turns(&msgs, 30);
        assert_eq!(turns.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["make it\n\n$ ls", "done"]);
        let req = CodeRequest { task: CodeTask::Suggest, instruction: "x".into(), files: Vec::new() };
        let m = CodeState::messages(&req, &turns).unwrap();
        assert_eq!(m[1].content, "make it\n\n$ ls", "the conversation comes right after the system prompt");
    }
}
