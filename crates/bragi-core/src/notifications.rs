//! The notification log: every notice a shell shows (its toast goes away; this keeps it), with
//! when and how it read, newest last, at most [`CAP`]. A shell can keep it in the profile
//! ([`NotificationLog::open`]) so it outlives the session, or in memory. It counts what came in
//! since the log was last looked at.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The most notices kept.
pub const CAP: usize = 200;

/// The log's file name in the profile.
pub const FILE: &str = "notifications.jsonl";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Info,
    Ok,
    Ask,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    /// Unix milliseconds.
    pub at_ms: i64,
    pub level: Level,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct NotificationLog {
    entries: Vec<Notification>,
    file: Option<PathBuf>,
    unread: usize,
}

impl NotificationLog {
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Loads the log in `profile` (missing: empty); new notices are appended to it.
    pub fn open(profile: &Path) -> Self {
        let file = profile.join(FILE);
        let entries = std::fs::read_to_string(&file)
            .map(|t| t.lines().filter_map(|l| serde_json::from_str::<Notification>(l).ok()).collect::<Vec<_>>())
            .unwrap_or_default();
        let mut log = Self { entries, file: Some(file), unread: 0 };
        log.trim();
        log
    }

    pub fn entries(&self) -> &[Notification] {
        &self.entries
    }

    /// Notices since [`seen`](Self::seen) was last called.
    pub fn unread(&self) -> usize {
        self.unread
    }

    pub fn seen(&mut self) {
        self.unread = 0;
    }

    /// Adds a notice (blank ones are skipped) stamped now.
    pub fn push(&mut self, level: Level, text: &str) {
        let at_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        self.push_at(at_ms, level, text);
    }

    pub fn push_at(&mut self, at_ms: i64, level: Level, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let n = Notification { at_ms, level, text: text.to_string() };
        self.entries.push(n.clone());
        self.unread += 1;
        let over = self.entries.len() > CAP;
        self.trim();
        if let Some(f) = &self.file {
            let _ = if over { self.rewrite(f) } else { append(f, &n) };
        }
    }

    /// Empties the log (and its file).
    pub fn clear(&mut self) {
        self.entries.clear();
        self.unread = 0;
        if let Some(f) = &self.file {
            let _ = self.rewrite(f);
        }
    }

    fn trim(&mut self) {
        if self.entries.len() > CAP {
            let cut = self.entries.len() - CAP;
            self.entries.drain(..cut);
        }
        self.unread = self.unread.min(self.entries.len());
    }

    fn rewrite(&self, f: &Path) -> std::io::Result<()> {
        if let Some(d) = f.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = f.with_extension("jsonl.tmp");
        let mut text = String::new();
        for e in &self.entries {
            text.push_str(&serde_json::to_string(e).unwrap_or_default());
            text.push('\n');
        }
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, f)
    }
}

fn append(f: &Path, n: &Notification) -> std::io::Result<()> {
    if let Some(d) = f.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(f)?;
    writeln!(file, "{}", serde_json::to_string(n).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_notices_counts_unread_and_persists_with_a_cap() {
        let d = tempfile::tempdir().unwrap();
        let mut log = NotificationLog::open(d.path());
        log.push(Level::Info, "Context: 3 files");
        log.push(Level::Error, "No node answered");
        log.push(Level::Info, "   ");
        assert_eq!(log.entries().len(), 2);
        assert_eq!(log.unread(), 2);
        log.seen();
        assert_eq!(log.unread(), 0);
        let again = NotificationLog::open(d.path());
        assert_eq!(again.entries()[1].text, "No node answered");
        assert_eq!(again.entries()[1].level, Level::Error);
        assert_eq!(again.unread(), 0, "a new session starts read");
        let mut big = NotificationLog::open(d.path());
        for i in 0..CAP + 3 {
            big.push_at(i as i64, Level::Info, &format!("n{i}"));
        }
        assert_eq!(big.entries().len(), CAP);
        assert_eq!(big.unread(), CAP);
        assert_eq!(NotificationLog::open(d.path()).entries().len(), CAP);
        big.clear();
        assert!(NotificationLog::open(d.path()).entries().is_empty());
    }
}
