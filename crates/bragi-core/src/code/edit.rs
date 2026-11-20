//! Writing accepted changes to disk, with undo. Each [`Journal::apply`] is one entry: a folder
//! in the journal holding what every file was before and what was written, so
//! [`Journal::undo_last`] can put the files back. Nothing is written unless every file still
//! holds what the diff was made from, and undo refuses to clobber a file edited since (unless
//! forced).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, ErrorKind, Result};
use crate::history::now_ms;

/// One file to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    pub path: PathBuf,
    /// What the file must hold now; `None`: it must not exist yet.
    pub before: Option<String>,
    pub after: String,
}

/// What an apply or undo touched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub id: String,
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ManifestFile {
    path: PathBuf,
    existed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Manifest {
    id: String,
    at_ms: i64,
    files: Vec<ManifestFile>,
    #[serde(default)]
    undone: bool,
}

/// One apply, as [`Journal::entries`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    pub id: String,
    pub at_ms: i64,
    pub files: Vec<PathBuf>,
    pub undone: bool,
}

/// The undo journal, in a folder of its own (the shells keep it in their data folder, never in
/// the user's project).
#[derive(Debug, Clone)]
pub struct Journal {
    dir: PathBuf,
}

fn storage(path: &Path, e: std::io::Error) -> CoreError {
    CoreError::new(ErrorKind::Storage, format!("{}: {e}", path.display()))
}

fn current(path: &Path) -> Result<Option<String>> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(String::from_utf8_lossy(&b).into_owned())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(storage(path, e)),
    }
}

/// Writes `text` to `path` through a temp file and a rename, keeping the file's permissions.
fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| storage(dir, e))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.bragi-{}", crate::core::random_id("w")));
    std::fs::write(&tmp, text).map_err(|e| storage(&tmp, e))?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        storage(path, e)
    })
}

impl Journal {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn manifest(&self, id: &str) -> Result<Manifest> {
        let p = self.dir.join(id).join("manifest.json");
        let text = std::fs::read_to_string(&p).map_err(|e| storage(&p, e))?;
        serde_json::from_str(&text).map_err(|e| CoreError::new(ErrorKind::Storage, format!("{}: {e}", p.display())))
    }

    fn save(&self, m: &Manifest) -> Result<()> {
        let p = self.dir.join(&m.id).join("manifest.json");
        write_atomic(&p, &serde_json::to_string_pretty(m).map_err(|e| CoreError::new(ErrorKind::Other, e.to_string()))?)
    }

    /// Writes every change, or none: each file must still hold `before`. Returns the entry.
    pub fn apply(&self, changes: &[FileChange]) -> Result<Applied> {
        if changes.is_empty() {
            return Err(CoreError::new(ErrorKind::BadRequest, "nothing to apply"));
        }
        for c in changes {
            if current(&c.path)? != c.before {
                return Err(CoreError::new(ErrorKind::BadRequest, format!("{} changed since the suggestion was made; ask again", c.path.display())));
            }
        }
        let id = format!("{:013}-{}", now_ms(), crate::core::random_id("e"));
        let entry = self.dir.join(&id);
        std::fs::create_dir_all(&entry).map_err(|e| storage(&entry, e))?;
        let mut m = Manifest { id: id.clone(), at_ms: now_ms(), files: Vec::new(), undone: false };
        for (i, c) in changes.iter().enumerate() {
            if let Some(b) = &c.before {
                std::fs::write(entry.join(format!("{i}.before")), b).map_err(|e| storage(&entry, e))?;
            }
            std::fs::write(entry.join(format!("{i}.after")), &c.after).map_err(|e| storage(&entry, e))?;
            m.files.push(ManifestFile { path: c.path.clone(), existed: c.before.is_some() });
        }
        self.save(&m)?;
        for (i, c) in changes.iter().enumerate() {
            if let Err(e) = write_atomic(&c.path, &c.after) {
                // Put back what was already written, then drop the entry.
                for done in &changes[..i] {
                    let _ = match &done.before {
                        Some(b) => write_atomic(&done.path, b),
                        None => std::fs::remove_file(&done.path).map_err(|e| storage(&done.path, e)),
                    };
                }
                let _ = std::fs::remove_dir_all(&entry);
                return Err(e);
            }
        }
        Ok(Applied { id, files: changes.iter().map(|c| c.path.clone()).collect() })
    }

    /// Every entry, newest first.
    pub fn entries(&self) -> Vec<JournalEntry> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut ids: Vec<String> = rd.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        ids.sort();
        ids.reverse();
        ids.into_iter()
            .filter_map(|id| self.manifest(&id).ok())
            .map(|m| JournalEntry { id: m.id, at_ms: m.at_ms, files: m.files.into_iter().map(|f| f.path).collect(), undone: m.undone })
            .collect()
    }

    /// Undoes the newest entry not undone yet.
    pub fn undo_last(&self, force: bool) -> Result<Applied> {
        let e = self.entries().into_iter().find(|e| !e.undone).ok_or_else(|| CoreError::new(ErrorKind::NotFound, "nothing to undo"))?;
        self.undo(&e.id, force)
    }

    /// Puts entry `id`'s files back. Without `force`, refuses when a file no longer holds what
    /// the entry wrote (it was edited since).
    pub fn undo(&self, id: &str, force: bool) -> Result<Applied> {
        let mut m = self.manifest(id)?;
        if m.undone {
            return Err(CoreError::new(ErrorKind::BadRequest, format!("{id} was already undone")));
        }
        let entry = self.dir.join(id);
        let read = |name: String| {
            let p = entry.join(name);
            std::fs::read_to_string(&p).map_err(|e| storage(&p, e))
        };
        if !force {
            for (i, f) in m.files.iter().enumerate() {
                if current(&f.path)?.as_deref() != Some(read(format!("{i}.after"))?.as_str()) {
                    return Err(CoreError::new(
                        ErrorKind::BadRequest,
                        format!("{} was edited after the change; undo anyway with force", f.path.display()),
                    ));
                }
            }
        }
        for (i, f) in m.files.iter().enumerate() {
            if f.existed {
                write_atomic(&f.path, &read(format!("{i}.before"))?)?;
            } else {
                match std::fs::remove_file(&f.path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(storage(&f.path, e)),
                }
            }
        }
        m.undone = true;
        self.save(&m)?;
        Ok(Applied { id: id.to_string(), files: m.files.into_iter().map(|f| f.path).collect() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::diff::FileDiff;

    #[test]
    fn apply_accepted_hunks_then_undo() {
        let d = tempfile::tempdir().unwrap();
        let (src, journal) = (d.path().join("a.txt"), Journal::new(d.path().join("journal")));
        let old: String = (1..=20).map(|i| format!("{i}\n")).collect();
        let old = old.as_str();
        std::fs::write(&src, old).unwrap();
        let diff = FileDiff::compute("a.txt", old, old.replacen("2\n", "two\n", 1).replace("19\n", "nineteen\n"));
        assert_eq!(diff.hunks.len(), 2);
        let after = diff.apply(&[true, false]);
        let applied = journal.apply(&[FileChange { path: src.clone(), before: Some(old.into()), after: after.clone() }]).unwrap();
        assert_eq!(std::fs::read_to_string(&src).unwrap(), old.replacen("2\n", "two\n", 1));
        assert_eq!(journal.entries().len(), 1);
        let undone = journal.undo_last(false).unwrap();
        assert_eq!(undone.id, applied.id);
        assert_eq!(std::fs::read_to_string(&src).unwrap(), old);
        assert!(journal.undo_last(false).is_err(), "nothing left to undo");
    }

    #[test]
    fn refuses_stale_writes_and_clobbering_undo() {
        let d = tempfile::tempdir().unwrap();
        let (src, journal) = (d.path().join("a.txt"), Journal::new(d.path().join("j")));
        std::fs::write(&src, "now\n").unwrap();
        let stale = FileChange { path: src.clone(), before: Some("then\n".into()), after: "x\n".into() };
        assert!(journal.apply(&[stale]).is_err());
        assert_eq!(std::fs::read_to_string(&src).unwrap(), "now\n");
        journal.apply(&[FileChange { path: src.clone(), before: Some("now\n".into()), after: "new\n".into() }]).unwrap();
        std::fs::write(&src, "edited by hand\n").unwrap();
        assert!(journal.undo_last(false).is_err());
        assert_eq!(std::fs::read_to_string(&src).unwrap(), "edited by hand\n");
        journal.undo_last(true).unwrap();
        assert_eq!(std::fs::read_to_string(&src).unwrap(), "now\n");
    }

    #[test]
    fn created_files_are_removed_on_undo_and_all_or_nothing() {
        let d = tempfile::tempdir().unwrap();
        let journal = Journal::new(d.path().join("j"));
        let (a, b) = (d.path().join("new/a.rs"), d.path().join("b.rs"));
        std::fs::write(&b, "b\n").unwrap();
        // b's precondition fails, so a isn't created either.
        let bad = [
            FileChange { path: a.clone(), before: None, after: "a\n".into() },
            FileChange { path: b.clone(), before: Some("not b\n".into()), after: "B\n".into() },
        ];
        assert!(journal.apply(&bad).is_err());
        assert!(!a.exists());
        let good = [
            FileChange { path: a.clone(), before: None, after: "a\n".into() },
            FileChange { path: b.clone(), before: Some("b\n".into()), after: "B\n".into() },
        ];
        journal.apply(&good).unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "a\n");
        journal.undo_last(false).unwrap();
        assert!(!a.exists());
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "b\n");
    }

    #[cfg(unix)]
    #[test]
    fn keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("run.sh");
        std::fs::write(&p, "echo 1\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        Journal::new(d.path().join("j")).apply(&[FileChange { path: p.clone(), before: Some("echo 1\n".into()), after: "echo 2\n".into() }]).unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o755);
    }
}
