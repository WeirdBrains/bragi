//! Code mode's state: the context, the request, the answer and the review of its diffs. All
//! the prompt, parsing, diff and undo logic is `bragi_core::code`, shared with the editor
//! extension; this only keeps track of what the user picked.

use std::path::{Path, PathBuf};

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
}

/// What a typed line in code mode asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Add(Vec<String>),
    Drop(String),
    Clear,
    Root(String),
    Ask(CodeRequest),
    Undo { force: bool },
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

    pub fn skipped_note(&self) -> String {
        match self.gathered.skipped.len() {
            0 => String::new(),
            n => format!(" ({n} left out: ignored, binary, secret or too big)"),
        }
    }

    /// Reads a typed line: `/add`, `/drop`, `/clear`, `/root`, `/rewrite FILE[:A-B] how`,
    /// `/suggest [what]`, `/undo[!]`; anything else asks for suggestions about it.
    pub fn parse(&mut self, line: &str) -> Result<Command, String> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(Command::Nothing);
        }
        let (cmd, rest) = match line.strip_prefix('/') {
            Some(c) => c.split_once(char::is_whitespace).map(|(a, b)| (a, b.trim())).unwrap_or((c, "")),
            None => ("suggest", line),
        };
        Ok(match cmd {
            "add" | "a" if !rest.is_empty() => Command::Add(rest.split_whitespace().map(str::to_string).collect()),
            "drop" | "d" if !rest.is_empty() => Command::Drop(rest.to_string()),
            "clear" => Command::Clear,
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
                if self.gathered.files.is_empty() {
                    return Err("add files or a folder first: /add src".into());
                }
                Command::Ask(CodeRequest { task: CodeTask::Suggest, instruction: rest.to_string(), files: self.gathered.files.clone() })
            }
            _ => return Err(format!("unknown command /{cmd}; F1 lists them")),
        })
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
    pub fn messages(req: &CodeRequest) -> Result<Vec<bragi_core::code::PromptMessage>, String> {
        prompt::build(req).map_err(|e| e.message)
    }
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
        assert!(s.parse("tidy it").is_err(), "suggest needs context");
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
}
