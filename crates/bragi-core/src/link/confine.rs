//! Keeping every path a linked phone names inside the project roots the desktop chose.
//!
//! A request names a root (by its index in the desktop's list) and a path relative to it. The
//! path must be relative, use `/` separators, and have no `..`, `.` prefixes that climb, drive
//! letters or NUL bytes. After that check, the deepest part of the path that exists is
//! canonicalized (resolving symlinks) and must still be under the canonical root, so a symlink
//! inside the project that points outside it can't be followed out.

use std::path::{Component, Path, PathBuf};

use crate::error::{CoreError, ErrorKind, Result};

fn refused(msg: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::Refused, msg)
}

/// A project root the desktop shares, canonicalized when it was added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    canonical: PathBuf,
}

impl Root {
    /// `dir` must exist and be a folder.
    pub fn new(dir: &Path) -> Result<Self> {
        let canonical = dir.canonicalize().map_err(|e| CoreError::new(ErrorKind::Storage, format!("{}: {e}", dir.display())))?;
        if !canonical.is_dir() {
            return Err(CoreError::new(ErrorKind::BadRequest, format!("{} isn't a folder", dir.display())));
        }
        Ok(Self { canonical })
    }

    pub fn path(&self) -> &Path {
        &self.canonical
    }

    /// The real path of `rel` under this root, refusing anything that would leave it. `rel` may
    /// be empty or `.` (the root itself). The file doesn't have to exist.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf> {
        let clean = clean_relative(rel)?;
        let full = self.canonical.join(&clean);
        // The deepest existing ancestor, resolved, must be inside the root.
        let mut existing = full.as_path();
        let mut rest: Vec<&std::ffi::OsStr> = Vec::new();
        loop {
            match std::fs::symlink_metadata(existing) {
                Ok(_) => break,
                Err(_) => {
                    let name = existing.file_name().ok_or_else(|| refused(format!("{rel}: outside the project")))?;
                    rest.push(name);
                    existing = existing.parent().ok_or_else(|| refused(format!("{rel}: outside the project")))?;
                }
            }
        }
        let real = existing.canonicalize().map_err(|e| CoreError::new(ErrorKind::Storage, format!("{rel}: {e}")))?;
        if !real.starts_with(&self.canonical) {
            return Err(refused(format!("{rel}: outside the project")));
        }
        let mut out = real;
        for name in rest.into_iter().rev() {
            out.push(name);
        }
        Ok(out)
    }

    /// `path` (a real path under this root) as the `/`-separated relative path requests use.
    pub fn relative(&self, path: &Path) -> String {
        crate::code::context::relative(&self.canonical, path)
    }
}

/// Checks `rel` is a plain relative path and returns it normalized (no `.` parts).
pub fn clean_relative(rel: &str) -> Result<PathBuf> {
    if rel.contains('\0') {
        return Err(refused("a path can't hold a NUL byte"));
    }
    if rel.contains('\\') {
        return Err(refused(format!("{rel}: use / in paths")));
    }
    let p = Path::new(rel);
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::CurDir => {}
            Component::ParentDir => return Err(refused(format!("{rel}: `..` isn't allowed"))),
            Component::RootDir | Component::Prefix(_) => return Err(refused(format!("{rel}: paths are relative to the project"))),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> (tempfile::TempDir, Root) {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("proj/src")).unwrap();
        std::fs::write(d.path().join("proj/src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(d.path().join("secret.txt"), "outside").unwrap();
        let r = Root::new(&d.path().join("proj")).unwrap();
        (d, r)
    }

    #[test]
    fn plain_paths_resolve_inside() {
        let (_d, r) = root();
        assert_eq!(r.resolve("src/main.rs").unwrap(), r.path().join("src/main.rs"));
        assert_eq!(r.resolve("").unwrap(), r.path());
        assert_eq!(r.resolve("./src").unwrap(), r.path().join("src"));
        // Not there yet: fine, as long as it would be inside.
        assert_eq!(r.resolve("new/dir/file.txt").unwrap(), r.path().join("new/dir/file.txt"));
        assert_eq!(r.relative(&r.path().join("src/main.rs")), "src/main.rs");
    }

    #[test]
    fn climbing_and_absolute_paths_are_refused() {
        let (_d, r) = root();
        for bad in ["../secret.txt", "src/../../secret.txt", "/etc/passwd", "src/\0x", "..\\secret.txt", ".."] {
            let e = r.resolve(bad).unwrap_err();
            assert_eq!(e.kind, ErrorKind::Refused, "{bad}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_out_of_the_root_are_refused() {
        let (d, r) = root();
        std::os::unix::fs::symlink(d.path().join("secret.txt"), r.path().join("link.txt")).unwrap();
        std::os::unix::fs::symlink(d.path(), r.path().join("up")).unwrap();
        assert_eq!(r.resolve("link.txt").unwrap_err().kind, ErrorKind::Refused);
        assert_eq!(r.resolve("up/secret.txt").unwrap_err().kind, ErrorKind::Refused);
        assert_eq!(r.resolve("up/new-file").unwrap_err().kind, ErrorKind::Refused);
        // A symlink that stays inside is fine.
        std::os::unix::fs::symlink(r.path().join("src"), r.path().join("code")).unwrap();
        assert_eq!(r.resolve("code/main.rs").unwrap(), r.path().join("src/main.rs"));
    }
}
