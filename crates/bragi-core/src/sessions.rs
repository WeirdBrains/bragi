//! Sessions (LF-11): which target each conversation is bound to, and the nested folders that
//! organize them. Kept per device in `sessions.json` next to the history (a conversation's
//! messages sync; its folder and target do not, because endpoints are per device). Deleting a
//! folder, an endpoint or a conversation never deletes history: conversations move up a level or
//! become unbound.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::core::{lock, random_id};
use crate::error::{CoreError, ErrorKind, Result};

pub const SESSIONS_FILE: &str = "sessions.json";
pub const MAX_FOLDER_DEPTH: usize = 8;

/// What a session talks to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// The Weirdnet pool.
    Pool,
    /// One endpoint and a model on it.
    Endpoint { id: String, model: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SessionMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
    /// The target was deleted and nothing replaced it: the next send must pick one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unbound: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    folders: Vec<Folder>,
    #[serde(default)]
    sessions: BTreeMap<String, SessionMeta>,
}

pub struct Sessions {
    path: Mutex<PathBuf>,
    data: Mutex<File>,
}

impl Sessions {
    pub fn open(dir: &Path) -> Self {
        let path = dir.join(SESSIONS_FILE);
        let data = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self { path: Mutex::new(path), data: Mutex::new(data) }
    }

    fn path(&self) -> PathBuf {
        lock(&self.path).clone()
    }

    /// Loads the sessions file in another pool's directory.
    pub fn switch_root(&self, dir: &Path) {
        let path = dir.join(SESSIONS_FILE);
        let data = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        *lock(&self.path) = path;
        *lock(&self.data) = data;
    }

    fn save(&self, d: &File) -> Result<()> {
        let path = self.path();
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let bytes = serde_json::to_vec_pretty(d).map_err(|e| CoreError::new(ErrorKind::Storage, e.to_string()))?;
        crate::identity::write_private_file(&path, &bytes)?;
        Ok(())
    }

    pub fn folders(&self) -> Vec<Folder> {
        lock(&self.data).folders.clone()
    }

    pub fn meta(&self, conversation: &str) -> SessionMeta {
        lock(&self.data).sessions.get(conversation).cloned().unwrap_or_default()
    }

    pub fn all(&self) -> BTreeMap<String, SessionMeta> {
        lock(&self.data).sessions.clone()
    }

    pub fn create_folder(&self, name: &str, parent: Option<&str>) -> Result<Folder> {
        let name = folder_name(name)?;
        let mut d = lock(&self.data);
        if let Some(p) = parent {
            if !d.folders.iter().any(|f| f.id == p) {
                return Err(no_folder(p));
            }
            if depth(&d.folders, Some(p)) >= MAX_FOLDER_DEPTH {
                return Err(CoreError::new(ErrorKind::BadRequest, format!("folders nest at most {MAX_FOLDER_DEPTH} deep")));
            }
        }
        let f = Folder { id: random_id("f"), name, parent: parent.map(str::to_string) };
        d.folders.push(f.clone());
        self.save(&d)?;
        Ok(f)
    }

    pub fn rename_folder(&self, id: &str, name: &str) -> Result<()> {
        let name = folder_name(name)?;
        let mut d = lock(&self.data);
        d.folders.iter_mut().find(|f| f.id == id).ok_or_else(|| no_folder(id))?.name = name;
        self.save(&d)
    }

    /// Moves a folder under `parent` (`None` = top level). Not into itself or its own subfolders.
    pub fn move_folder(&self, id: &str, parent: Option<&str>) -> Result<()> {
        let mut d = lock(&self.data);
        if !d.folders.iter().any(|f| f.id == id) {
            return Err(no_folder(id));
        }
        if let Some(p) = parent {
            if !d.folders.iter().any(|f| f.id == p) {
                return Err(no_folder(p));
            }
            let mut at = Some(p.to_string());
            while let Some(a) = at {
                if a == id {
                    return Err(CoreError::new(ErrorKind::BadRequest, "a folder can't go inside itself"));
                }
                at = d.folders.iter().find(|f| f.id == a).and_then(|f| f.parent.clone());
            }
        }
        // The deepest branch under `id` must still fit.
        let below = subtree_height(&d.folders, id);
        if depth(&d.folders, parent) + 1 + below > MAX_FOLDER_DEPTH {
            return Err(CoreError::new(ErrorKind::BadRequest, format!("folders nest at most {MAX_FOLDER_DEPTH} deep")));
        }
        d.folders.iter_mut().find(|f| f.id == id).expect("checked").parent = parent.map(str::to_string);
        self.save(&d)
    }

    /// Deletes a folder. Its subfolders and conversations move up to its parent; nothing else goes.
    pub fn delete_folder(&self, id: &str) -> Result<()> {
        let mut d = lock(&self.data);
        let parent = d.folders.iter().find(|f| f.id == id).ok_or_else(|| no_folder(id))?.parent.clone();
        d.folders.retain(|f| f.id != id);
        for f in d.folders.iter_mut().filter(|f| f.parent.as_deref() == Some(id)) {
            f.parent = parent.clone();
        }
        for m in d.sessions.values_mut().filter(|m| m.folder.as_deref() == Some(id)) {
            m.folder = parent.clone();
        }
        self.save(&d)
    }

    pub fn set_folder(&self, conversation: &str, folder: Option<&str>) -> Result<()> {
        let mut d = lock(&self.data);
        if let Some(f) = folder.filter(|f| !d.folders.iter().any(|x| x.id == *f)) {
            return Err(no_folder(f));
        }
        d.sessions.entry(conversation.to_string()).or_default().folder = folder.map(str::to_string);
        self.save(&d)
    }

    /// Binds a session to `target` (clears `unbound`).
    pub fn bind(&self, conversation: &str, target: Target) -> Result<()> {
        let mut d = lock(&self.data);
        let m = d.sessions.entry(conversation.to_string()).or_default();
        m.target = Some(target);
        m.unbound = false;
        self.save(&d)
    }

    /// Sessions bound to endpoint `id`: moved to `to`, or marked unbound. Returns how many.
    pub fn rebind_endpoint(&self, id: &str, to: Option<&Target>) -> Result<usize> {
        let mut d = lock(&self.data);
        let mut n = 0;
        for m in d.sessions.values_mut() {
            if matches!(&m.target, Some(Target::Endpoint { id: e, .. }) if e == id) {
                match to {
                    Some(t) => m.target = Some(t.clone()),
                    None => {
                        m.target = None;
                        m.unbound = true;
                    }
                }
                n += 1;
            }
        }
        if n > 0 {
            self.save(&d)?;
        }
        Ok(n)
    }

    /// How many sessions are bound to endpoint `id`.
    pub fn bound_to(&self, id: &str) -> usize {
        lock(&self.data).sessions.values().filter(|m| matches!(&m.target, Some(Target::Endpoint { id: e, .. }) if e == id)).count()
    }
}

fn folder_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > 60 {
        return Err(CoreError::new(ErrorKind::BadRequest, "give the folder a name (up to 60 characters)"));
    }
    Ok(n.to_string())
}

fn no_folder(id: &str) -> CoreError {
    CoreError::new(ErrorKind::NotFound, format!("no folder `{id}`"))
}

/// Levels from the top down to and including `folder` (0 for the top level).
fn depth(folders: &[Folder], folder: Option<&str>) -> usize {
    let mut n = 0;
    let mut at = folder.map(str::to_string);
    while let Some(a) = at {
        n += 1;
        if n > 64 {
            break;
        }
        at = folders.iter().find(|f| f.id == a).and_then(|f| f.parent.clone());
    }
    n
}

/// Levels below `id` (0 when it has no subfolders).
fn subtree_height(folders: &[Folder], id: &str) -> usize {
    folders.iter().filter(|f| f.parent.as_deref() == Some(id)).map(|f| 1 + subtree_height(folders, &f.id)).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_nest_move_and_delete_without_losing_sessions() {
        let d = tempfile::tempdir().unwrap();
        let s = Sessions::open(d.path());
        let a = s.create_folder("Work", None).unwrap();
        let b = s.create_folder("Reports", Some(&a.id)).unwrap();
        s.set_folder("c-1", Some(&b.id)).unwrap();
        assert!(s.move_folder(&a.id, Some(&b.id)).is_err(), "no cycles");
        assert!(s.create_folder("x", Some("f-nope")).is_err());
        s.delete_folder(&b.id).unwrap();
        assert_eq!(s.meta("c-1").folder.as_deref(), Some(a.id.as_str()), "moves up");
        s.delete_folder(&a.id).unwrap();
        assert_eq!(s.meta("c-1").folder, None);
        assert!(Sessions::open(d.path()).folders().is_empty());
    }

    #[test]
    fn depth_is_limited() {
        let d = tempfile::tempdir().unwrap();
        let s = Sessions::open(d.path());
        let mut parent: Option<String> = None;
        for i in 0..MAX_FOLDER_DEPTH {
            parent = Some(s.create_folder(&format!("l{i}"), parent.as_deref()).unwrap().id);
        }
        assert!(s.create_folder("too deep", parent.as_deref()).is_err());
    }

    #[test]
    fn rebind_or_unbind() {
        let d = tempfile::tempdir().unwrap();
        let s = Sessions::open(d.path());
        s.bind("c-1", Target::Endpoint { id: "ep-1".into(), model: "m".into() }).unwrap();
        s.bind("c-2", Target::Endpoint { id: "ep-1".into(), model: "m".into() }).unwrap();
        s.bind("c-3", Target::Pool).unwrap();
        assert_eq!(s.bound_to("ep-1"), 2);
        assert_eq!(s.rebind_endpoint("ep-1", Some(&Target::Pool)).unwrap(), 2);
        assert_eq!(s.meta("c-1").target, Some(Target::Pool));
        s.bind("c-4", Target::Endpoint { id: "ep-2".into(), model: "m".into() }).unwrap();
        s.rebind_endpoint("ep-2", None).unwrap();
        assert!(s.meta("c-4").unbound && s.meta("c-4").target.is_none());
        s.bind("c-4", Target::Pool).unwrap();
        assert!(!s.meta("c-4").unbound);
    }
}
