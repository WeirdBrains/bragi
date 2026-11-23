//! The code tools on a project folder, every path confined to a [`Root`]: what the desktop link
//! serves to a phone, and what a shell with its own file system (the Flutter app on a computer)
//! runs locally. Both go through these functions, so a phone and a desktop see the same rules.

use std::path::PathBuf;

use crate::code::{FileChange, Journal, Limits, gather as gather_files};
use crate::error::{CoreError, ErrorKind, Result};

use super::confine::Root;
use super::proto::{AppliedReply, Entry, GatherReply, ProjectReply, WireChange};

/// The largest file [`read`] hands over.
pub const MAX_READ: u64 = 2 * 1024 * 1024;

/// A folder's entries, folders first, then by name. `.git` is left out.
pub fn list(root: &Root, rel: &str) -> Result<Vec<Entry>> {
    let dir = root.resolve(rel)?;
    let rd = std::fs::read_dir(&dir).map_err(|e| CoreError::new(ErrorKind::Storage, format!("{rel}: {e}")))?;
    let mut out: Vec<Entry> = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        // Follows symlinks for the kind, but a link out of the root isn't listed.
        let rel_child = if rel.trim_matches('/').is_empty() { name.clone() } else { format!("{}/{name}", rel.trim_end_matches('/')) };
        let Ok(real) = root.resolve(&rel_child) else { continue };
        let Ok(meta) = std::fs::metadata(&real) else { continue };
        out.push(Entry { name, dir: meta.is_dir(), size: if meta.is_dir() { 0 } else { meta.len() } });
    }
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

/// A text file (`None` when there's nothing at the path). Likely secrets, binaries and files
/// over [`MAX_READ`] are refused.
pub fn read(root: &Root, rel: &str) -> Result<Option<String>> {
    let path = root.resolve(rel)?;
    if crate::code::context::looks_secret(&path) {
        return Err(CoreError::new(ErrorKind::Refused, format!("{rel} looks like a secret; it isn't sent over the link")));
    }
    let meta = match std::fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(CoreError::new(ErrorKind::Storage, format!("{rel}: {e}"))),
    };
    if meta.is_dir() {
        return Err(CoreError::new(ErrorKind::BadRequest, format!("{rel} is a folder")));
    }
    if meta.len() > MAX_READ {
        return Err(CoreError::new(ErrorKind::BadRequest, format!("{rel} is over {} MiB", MAX_READ / 1024 / 1024)));
    }
    let bytes = std::fs::read(&path).map_err(|e| CoreError::new(ErrorKind::Storage, format!("{rel}: {e}")))?;
    if bytes.contains(&0) {
        return Err(CoreError::new(ErrorKind::BadRequest, format!("{rel} is a binary file")));
    }
    String::from_utf8(bytes).map(Some).map_err(|_| CoreError::new(ErrorKind::BadRequest, format!("{rel} isn't UTF-8 text")))
}

/// The files and folders `targets` name, read as code context (see [`crate::code::gather`]).
pub fn gather(root: &Root, targets: &[String]) -> Result<GatherReply> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for t in targets {
        paths.push(root.resolve(t)?);
    }
    let g = gather_files(root.path(), &paths, Limits::default())?;
    Ok(GatherReply { files: g.files, skipped: g.skipped })
}

/// Writes every change or none (each file must still hold `before`), keeping an undo entry in
/// `journal`.
pub fn apply(root: &Root, journal: &Journal, changes: &[WireChange]) -> Result<AppliedReply> {
    let mut out = Vec::new();
    for c in changes {
        let path = root.resolve(&c.path)?;
        if crate::code::context::looks_secret(&path) {
            return Err(CoreError::new(ErrorKind::Refused, format!("{} looks like a secret; the link doesn't write it", c.path)));
        }
        out.push(FileChange { path, before: c.before.clone(), after: c.after.clone() });
    }
    let a = journal.apply(&out)?;
    Ok(AppliedReply { id: a.id, files: a.files.iter().map(|p| root.relative(p)).collect() })
}

/// Puts back the newest change written through `journal`. Refuses when one of its files now
/// resolves outside the root (a symlink put in its place).
pub fn undo(root: &Root, journal: &Journal, force: bool) -> Result<AppliedReply> {
    let e = journal.entries().into_iter().find(|e| !e.undone).ok_or_else(|| CoreError::new(ErrorKind::NotFound, "nothing to undo"))?;
    for f in &e.files {
        if root.resolve(&root.relative(f))? != *f {
            return Err(CoreError::new(ErrorKind::Refused, format!("{} moved out of the project", f.display())));
        }
    }
    let a = journal.undo(&e.id, force)?;
    Ok(AppliedReply { id: a.id, files: a.files.iter().map(|p| root.relative(p)).collect() })
}

/// Opens `rel` as a project: makes the folder when it's missing and writes the starter files
/// into an empty one ([`crate::project::ensure`]).
pub fn project(root: &Root, rel: &str) -> Result<ProjectReply> {
    let dir = root.resolve(rel)?;
    let started = crate::project::ensure(&dir)?;
    let shown = root.relative(&dir);
    let shown = if shown.is_empty() { ".".to_string() } else { shown };
    Ok(ProjectReply { note: started.note(std::path::Path::new(&shown)), path: shown })
}

/// The undo journal for one root, under `dir` (the desktop's own data, never the project).
pub fn journal_for(dir: &std::path::Path, root: &Root) -> Journal {
    let h = blake3::hash(root.path().to_string_lossy().as_bytes());
    Journal::new(dir.join(super::proto::hex(&h.as_bytes()[..8])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_read_apply_undo_stay_in_the_root() {
        let d = tempfile::tempdir().unwrap();
        let proj = d.path().join("proj");
        std::fs::create_dir_all(proj.join("src")).unwrap();
        std::fs::create_dir_all(proj.join(".git")).unwrap();
        std::fs::write(proj.join("src/a.rs"), "one\n").unwrap();
        std::fs::write(proj.join(".env"), "KEY=1").unwrap();
        let root = Root::new(&proj).unwrap();
        let names: Vec<String> = list(&root, "").unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["src", ".env"]);
        assert_eq!(read(&root, "src/a.rs").unwrap().as_deref(), Some("one\n"));
        assert_eq!(read(&root, "nope.rs").unwrap(), None);
        assert_eq!(read(&root, ".env").unwrap_err().kind, ErrorKind::Refused);
        assert_eq!(read(&root, "../x").unwrap_err().kind, ErrorKind::Refused);
        let j = journal_for(&d.path().join("journal"), &root);
        let changes = vec![
            WireChange { path: "src/a.rs".into(), before: Some("one\n".into()), after: "two\n".into() },
            WireChange { path: "src/new/b.rs".into(), before: None, after: "b\n".into() },
        ];
        let a = apply(&root, &j, &changes).unwrap();
        assert_eq!(a.files, ["src/a.rs", "src/new/b.rs"]);
        assert_eq!(std::fs::read_to_string(proj.join("src/a.rs")).unwrap(), "two\n");
        // Stale `before`: nothing is written.
        assert!(apply(&root, &j, &changes).is_err());
        assert!(apply(&root, &j, &[WireChange { path: "../out.txt".into(), before: None, after: "x".into() }]).is_err());
        assert!(!d.path().join("out.txt").exists());
        undo(&root, &j, false).unwrap();
        assert_eq!(std::fs::read_to_string(proj.join("src/a.rs")).unwrap(), "one\n");
        assert!(!proj.join("src/new/b.rs").exists());
        let g = gather(&root, &["src".into()]).unwrap();
        assert_eq!(g.files.len(), 1);
        let p = project(&root, "fresh").unwrap();
        assert_eq!(p.path, "fresh");
        assert!(proj.join("fresh/README.md").exists() && !p.note.is_empty());
    }
}
