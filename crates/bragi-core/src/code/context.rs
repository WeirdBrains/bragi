//! The files a code request is about: what the user named, plus the files under any directory
//! they named that git would track. Ignored files (`.gitignore`, `.ignore`, the global git
//! excludes), hidden files, binaries, likely secrets and oversize files are left out of
//! directories and listed in [`Gathered::skipped`] with the reason. A file named on its own is
//! read even when ignored, but never when it looks like a secret.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, ErrorKind, Result};

/// One file as the model sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextFile {
    /// The path relative to the root, with `/` separators (what prompts and diffs show).
    pub path: String,
    pub text: String,
}

/// How much context one request may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_files: usize,
    pub max_file_bytes: usize,
    pub max_total_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_files: 64, max_file_bytes: 256 * 1024, max_total_bytes: 512 * 1024 }
    }
}

/// What [`gather`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Gathered {
    pub files: Vec<ContextFile>,
    /// `(path, why)` for every file that was left out.
    pub skipped: Vec<(String, String)>,
}

impl Gathered {
    pub fn total_bytes(&self) -> usize {
        self.files.iter().map(|f| f.text.len()).sum()
    }

    pub fn get(&self, path: &str) -> Option<&ContextFile> {
        self.files.iter().find(|f| f.path == path)
    }
}

/// File names that usually hold credentials. Never read as context, even when named.
pub fn looks_secret(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.ends_with(".p12")
        || name.ends_with(".pfx")
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name.starts_with("id_ecdsa")
        || name == ".netrc"
        || name == ".npmrc"
        || name == ".pypirc"
        || name == "credentials"
        || name.ends_with(".keystore")
        || name.ends_with(".jks")
}

/// `path` relative to `root` with `/` separators, or as given when it is outside `root`.
pub fn relative(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            Component::ParentDir => Some("..".into()),
            _ => None,
        })
        .collect();
    if parts.is_empty() { rel.to_string_lossy().into_owned() } else { parts.join("/") }
}

/// Reads `target` (relative to `root` unless absolute) and, for directories, every file under it
/// that git would track. Order is stable: targets in the order given, files sorted by path.
pub fn gather(root: &Path, targets: &[PathBuf], limits: Limits) -> Result<Gathered> {
    let mut out = Gathered::default();
    let mut total = 0usize;
    for t in targets {
        let path = if t.is_absolute() { t.clone() } else { root.join(t) };
        let meta = std::fs::metadata(&path).map_err(|e| CoreError::new(ErrorKind::Storage, format!("{}: {e}", relative(root, &path))))?;
        let files: Vec<(PathBuf, bool)> = if meta.is_dir() {
            let mut found = Vec::new();
            let walk = ignore::WalkBuilder::new(&path)
                .hidden(true)
                .git_ignore(true)
                .git_global(true)
                .git_exclude(true)
                .ignore(true)
                .parents(true)
                .require_git(false)
                .sort_by_file_name(|a, b| a.cmp(b))
                .build();
            for entry in walk.flatten() {
                if entry.file_type().is_some_and(|t| t.is_file()) {
                    found.push((entry.into_path(), false));
                }
            }
            found
        } else {
            vec![(path.clone(), true)]
        };
        for (file, named) in files {
            let rel = relative(root, &file);
            if out.files.iter().any(|f| f.path == rel) {
                continue;
            }
            let skip = |out: &mut Gathered, why: &str| out.skipped.push((rel.clone(), why.to_string()));
            if looks_secret(&file) {
                skip(&mut out, "looks like a secret");
                continue;
            }
            if out.files.len() >= limits.max_files {
                skip(&mut out, "too many files");
                continue;
            }
            let len = std::fs::metadata(&file).map(|m| m.len() as usize).unwrap_or(0);
            if len > limits.max_file_bytes {
                skip(&mut out, "too big");
                continue;
            }
            if total + len > limits.max_total_bytes {
                skip(&mut out, "over the context budget");
                continue;
            }
            let bytes = match std::fs::read(&file) {
                Ok(b) => b,
                Err(e) if named => return Err(CoreError::new(ErrorKind::Storage, format!("{rel}: {e}"))),
                Err(_) => {
                    skip(&mut out, "unreadable");
                    continue;
                }
            };
            if bytes.contains(&0) {
                skip(&mut out, "binary");
                continue;
            }
            let Ok(text) = String::from_utf8(bytes) else {
                skip(&mut out, "not UTF-8");
                continue;
            };
            total += text.len();
            out.files.push(ContextFile { path: rel, text });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respects_gitignore_and_skips_secrets_and_binaries() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        std::fs::write(root.join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn x() {}\n").unwrap();
        std::fs::write(root.join("target/out.rs"), "junk").unwrap();
        std::fs::write(root.join("run.log"), "log").unwrap();
        std::fs::write(root.join(".env"), "TOKEN=nope").unwrap();
        std::fs::write(root.join("blob.bin"), [0u8, 1, 2]).unwrap();
        let g = gather(root, &[PathBuf::from(".")], Limits::default()).unwrap();
        let paths: Vec<&str> = g.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["src/lib.rs", "src/main.rs"]);
        assert!(g.skipped.iter().any(|(p, why)| p == "blob.bin" && why == "binary"));
        // Named on its own: an ignored file is read, a secret never.
        let g = gather(root, &[PathBuf::from("run.log"), PathBuf::from(".env")], Limits::default()).unwrap();
        assert_eq!(g.files.len(), 1);
        assert_eq!(g.files[0].path, "run.log");
        assert_eq!(g.skipped, vec![(".env".to_string(), "looks like a secret".to_string())]);
    }

    #[test]
    fn limits_are_kept() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(d.path().join(format!("f{i}.txt")), "x".repeat(100)).unwrap();
        }
        let g = gather(d.path(), &[PathBuf::from(".")], Limits { max_files: 3, max_file_bytes: 1000, max_total_bytes: 250 }).unwrap();
        assert_eq!(g.files.len(), 2);
        assert_eq!(g.skipped.len(), 3);
        assert!(g.total_bytes() <= 250);
    }
}
