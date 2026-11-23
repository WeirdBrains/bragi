//! Prompt history, shell style: Up recalls the prompt sent before, Down the one after, and going
//! Down past the newest brings back what was being typed. Each project folder has its own
//! history (a global one when there is no project), kept on this device in the profile as JSON
//! lines, newest last, at most [`CAP`] entries. A prompt the same as the one before isn't added
//! again. Histories are local: they are never synced.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The most prompts one history keeps.
pub const CAP: usize = 1000;

/// The folder, under the profile, the history files live in.
pub const DIR: &str = "prompt-history";

#[derive(Debug, Clone, Default)]
pub struct PromptHistory {
    entries: Vec<String>,
    file: Option<PathBuf>,
    /// Where Up/Down are: an index into `entries`, or `None` at the draft.
    at: Option<usize>,
    /// What was typed before the first Up.
    draft: String,
}

/// The history file for `project` (or the global one) under `profile`.
pub fn file_for(profile: &Path, project: Option<&Path>) -> PathBuf {
    let name = match project {
        None => "global.jsonl".to_string(),
        Some(p) => {
            let p = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
            // FNV-1a of the path: stable, and keeps the folder name out of the file name.
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for b in p.to_string_lossy().bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
            format!("project-{h:016x}.jsonl")
        }
    };
    profile.join(DIR).join(name)
}

impl PromptHistory {
    /// A history kept only in memory.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Loads `file` (missing or unreadable: empty); new prompts are appended to it.
    pub fn open(file: PathBuf) -> Self {
        let entries = std::fs::read_to_string(&file)
            .map(|t| t.lines().filter_map(|l| serde_json::from_str::<String>(l).ok()).filter(|s| !s.trim().is_empty()).collect::<Vec<_>>())
            .unwrap_or_default();
        let mut h = Self { entries, file: Some(file), ..Self::default() };
        h.trim();
        h
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Whether Up/Down are on a recalled entry rather than the draft.
    pub fn browsing(&self) -> bool {
        self.at.is_some()
    }

    /// Records a sent prompt (blank ones and a repeat of the last are skipped) and goes back to
    /// an empty draft.
    pub fn push(&mut self, prompt: &str) {
        self.at = None;
        self.draft.clear();
        if prompt.trim().is_empty() || self.entries.last().map(String::as_str) == Some(prompt) {
            return;
        }
        self.entries.push(prompt.to_string());
        let over = self.entries.len() > CAP;
        self.trim();
        if let Some(f) = &self.file {
            let _ = if over { self.rewrite(f) } else { append(f, prompt) };
        }
    }

    /// Up: the prompt before the one shown. `current` is what the box holds now (kept as the
    /// draft on the first Up). `None` when there is nothing older.
    pub fn up(&mut self, current: &str) -> Option<String> {
        let next = match self.at {
            None if self.entries.is_empty() => return None,
            None => {
                self.draft = current.to_string();
                self.entries.len() - 1
            }
            Some(0) => return None,
            Some(i) => i - 1,
        };
        self.at = Some(next);
        Some(self.entries[next].clone())
    }

    /// Down: the prompt after the one shown, or the draft past the newest. `None` when already
    /// at the draft.
    pub fn down(&mut self) -> Option<String> {
        let i = self.at?;
        if i + 1 < self.entries.len() {
            self.at = Some(i + 1);
            Some(self.entries[i + 1].clone())
        } else {
            self.at = None;
            Some(std::mem::take(&mut self.draft))
        }
    }

    /// Leaves browsing (the user edited a recalled prompt; it is the draft now).
    pub fn reset(&mut self) {
        self.at = None;
        self.draft.clear();
    }

    fn trim(&mut self) {
        if self.entries.len() > CAP {
            let cut = self.entries.len() - CAP;
            self.entries.drain(..cut);
        }
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

fn append(f: &Path, prompt: &str) -> std::io::Result<()> {
    if let Some(d) = f.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(f)?;
    writeln!(file, "{}", serde_json::to_string(prompt).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn up_and_down_walk_the_history_and_bring_back_the_draft() {
        let mut h = PromptHistory::in_memory();
        assert_eq!(h.up("typing"), None);
        h.push("one");
        h.push("two");
        h.push("two");
        h.push("  ");
        h.push("three");
        assert_eq!(h.entries(), ["one", "two", "three"]);
        assert_eq!(h.down(), None, "already at the draft");
        assert_eq!(h.up("half-typed").as_deref(), Some("three"));
        assert_eq!(h.up("three").as_deref(), Some("two"));
        assert_eq!(h.up("two").as_deref(), Some("one"));
        assert_eq!(h.up("one"), None, "nothing older");
        assert!(h.browsing());
        assert_eq!(h.down().as_deref(), Some("two"));
        assert_eq!(h.down().as_deref(), Some("three"));
        assert_eq!(h.down().as_deref(), Some("half-typed"));
        assert!(!h.browsing());
        assert_eq!(h.down(), None);
        // Sending resets to an empty draft; a repeat of the last isn't added.
        h.up("");
        h.push("three");
        assert_eq!(h.entries().len(), 3);
        assert_eq!(h.up("").as_deref(), Some("three"));
    }

    #[test]
    fn it_persists_per_project_and_keeps_the_cap() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let b = d.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let fa = file_for(d.path(), Some(&a));
        let fb = file_for(d.path(), Some(&b));
        let fg = file_for(d.path(), None);
        assert!(fa != fb && fa != fg && fg.ends_with("prompt-history/global.jsonl"));
        let mut h = PromptHistory::open(fa.clone());
        h.push("multi\nline \"quoted\"");
        h.push("second");
        let h2 = PromptHistory::open(fa.clone());
        assert_eq!(h2.entries(), ["multi\nline \"quoted\"", "second"]);
        assert!(PromptHistory::open(fb).entries().is_empty());
        let mut big = PromptHistory::open(fg.clone());
        for i in 0..CAP + 5 {
            big.push(&format!("p{i}"));
        }
        assert_eq!(big.entries().len(), CAP);
        let again = PromptHistory::open(fg);
        assert_eq!(again.entries().len(), CAP);
        assert_eq!(again.entries()[0], "p5");
        assert_eq!(again.entries().last().map(String::as_str), Some(&*format!("p{}", CAP + 4)));
    }
}
