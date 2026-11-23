//! Starting a project: when the folder a shell opens for code work doesn't exist or is empty,
//! Bragi makes it and writes a small starter set ([`STARTER`]) so there is something to work
//! from. Existing files are never touched: a file is only written when nothing is at its path,
//! and a folder that has anything in it gets nothing.

use std::path::{Path, PathBuf};

use crate::error::{CoreError, ErrorKind, Result};

/// The starter files, relative to the project folder. `{name}` is the folder's name.
pub const STARTER: &[(&str, &str)] = &[
    ("README.md", "# {name}\n\nA new project. Say here what it is for and how to run it.\n"),
    (".gitignore", "# Local secrets and noise. Bragi never sends these to a model.\n.env\n.env.*\n*.log\n.DS_Store\n"),
];

/// What [`ensure`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Started {
    /// The folder didn't exist and was made.
    pub created_dir: bool,
    /// Starter files written, relative to the folder.
    pub files: Vec<String>,
}

impl Started {
    /// Whether anything was made.
    pub fn is_new(&self) -> bool {
        self.created_dir || !self.files.is_empty()
    }

    /// One line for a shell: empty when nothing was made.
    pub fn note(&self, dir: &Path) -> String {
        match (self.created_dir, self.files.is_empty()) {
            (_, true) if !self.created_dir => String::new(),
            (true, _) => format!("New project {}: created the folder and {}", dir.display(), list(&self.files)),
            (false, _) => format!("{} was empty: added {}", dir.display(), list(&self.files)),
        }
    }
}

fn list(files: &[String]) -> String {
    if files.is_empty() { "no files".into() } else { files.join(", ") }
}

/// Makes `dir` (and its parents) when it doesn't exist, and writes [`STARTER`] into it when it
/// is new or empty. A path that exists but isn't a folder is an error; a folder with anything
/// in it (hidden files included) is left as it is.
pub fn ensure(dir: &Path) -> Result<Started> {
    let storage = |p: &Path, e: std::io::Error| CoreError::new(ErrorKind::Storage, format!("{}: {e}", p.display()));
    let mut out = Started::default();
    match std::fs::metadata(dir) {
        Ok(m) if !m.is_dir() => return Err(CoreError::new(ErrorKind::BadRequest, format!("{} is a file, not a folder", dir.display()))),
        Ok(_) => {
            let mut entries = std::fs::read_dir(dir).map_err(|e| storage(dir, e))?;
            if entries.next().is_some() {
                return Ok(out);
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(dir).map_err(|e| storage(dir, e))?;
            out.created_dir = true;
        }
        Err(e) => return Err(storage(dir, e)),
    }
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Project".into());
    for (rel, body) in STARTER {
        let path: PathBuf = dir.join(rel);
        // create_new: never replace a file, even one that appeared since the check above.
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(body.replace("{name}", &name).as_bytes()).map_err(|e| storage(&path, e))?;
                out.files.push((*rel).to_string());
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(storage(&path, e)),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_folder_is_made_with_the_starter_files() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("deep/new-app");
        let s = ensure(&dir).unwrap();
        assert!(s.created_dir && s.is_new());
        assert_eq!(s.files, ["README.md", ".gitignore"]);
        assert_eq!(std::fs::read_to_string(dir.join("README.md")).unwrap().lines().next(), Some("# new-app"));
        assert!(std::fs::read_to_string(dir.join(".gitignore")).unwrap().contains(".env\n"));
        assert!(s.note(&dir).contains("created the folder and README.md, .gitignore"));
    }

    #[test]
    fn an_empty_folder_gets_the_starter_files() {
        let d = tempfile::tempdir().unwrap();
        let s = ensure(d.path()).unwrap();
        assert!(!s.created_dir);
        assert_eq!(s.files.len(), STARTER.len());
        assert!(s.note(d.path()).contains("was empty"));
    }

    #[test]
    fn a_folder_with_anything_in_it_is_left_alone() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(".hidden"), "keep").unwrap();
        let s = ensure(d.path()).unwrap();
        assert_eq!(s, Started::default());
        assert!(s.note(d.path()).is_empty());
        assert!(!d.path().join("README.md").exists());
        // Running it again on a started project changes nothing either.
        let p = d.path().join("p");
        ensure(&p).unwrap();
        std::fs::write(p.join("README.md"), "mine").unwrap();
        assert_eq!(ensure(&p).unwrap(), Started::default());
        assert_eq!(std::fs::read_to_string(p.join("README.md")).unwrap(), "mine");
    }

    #[test]
    fn a_file_is_not_a_project() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("notes.txt");
        std::fs::write(&f, "x").unwrap();
        let e = ensure(&f).unwrap_err();
        assert_eq!(e.kind, ErrorKind::BadRequest);
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "x");
    }
}
