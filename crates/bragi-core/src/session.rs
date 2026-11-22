//! Sessions: each sends to one target, the Weirdnet pool or one configured endpoint
//! ([`crate::endpoint`]), with its own model and its own conversations. Sessions sit in nested
//! folders the user arranges.
//!
//! Kept on this device only, in `<dir>/sessions.json`. A conversation records its session in its
//! metadata; one without a session (older history, or one from another device whose session this
//! device doesn't have) belongs to the default Weirdnet session, which can't be deleted.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::endpoint::{clean_name, write_file};
use crate::error::{CoreError, ErrorKind, Result};
use crate::history::now_ms;

pub const SESSIONS_FILE: &str = "sessions.json";
/// The session every device has: the Weirdnet pool, holding history from before sessions.
pub const DEFAULT_SESSION: &str = "weirdnet";

/// Where a session sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    /// The user's Weirdnet inference nodes (with the fallback endpoint, if one is set).
    Weirdnet,
    /// One configured endpoint, and only it.
    Endpoint { id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub target: Target,
    /// The model it asks for; the target's default when unset.
    #[serde(default)]
    pub model: Option<String>,
    /// The folder it is in; the top level when unset.
    #[serde(default)]
    pub folder: Option<String>,
    pub created_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub name: String,
    /// The folder it is in; the top level when unset.
    #[serde(default)]
    pub parent: Option<String>,
    /// Drawn closed.
    #[serde(default)]
    pub collapsed: bool,
}

/// The sessions, their folders and the open one: what the shells draw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sessions {
    #[serde(default)]
    pub folders: Vec<Folder>,
    #[serde(default)]
    pub sessions: Vec<Session>,
    #[serde(default = "default_id")]
    pub active: String,
}

fn default_id() -> String {
    DEFAULT_SESSION.into()
}

impl Default for Sessions {
    fn default() -> Self {
        Self { folders: Vec::new(), sessions: vec![default_session()], active: default_id() }
    }
}

fn default_session() -> Session {
    Session { id: default_id(), name: "Weirdnet".into(), target: Target::Weirdnet, model: None, folder: None, created_ms: 0 }
}

fn bad(msg: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::BadRequest, msg)
}

impl Sessions {
    pub fn get(&self, id: &str) -> Option<&Session> {
        self.sessions.iter().find(|s| s.id == id)
    }

    pub fn active(&self) -> &Session {
        self.get(&self.active).or_else(|| self.get(DEFAULT_SESSION)).unwrap_or(&self.sessions[0])
    }

    pub fn folder(&self, id: &str) -> Option<&Folder> {
        self.folders.iter().find(|f| f.id == id)
    }

    /// The session a conversation with `session` belongs to here.
    pub fn owner(&self, session: Option<&str>) -> &str {
        session.and_then(|s| self.get(s)).map_or(DEFAULT_SESSION, |s| s.id.as_str())
    }

    /// Folders and sessions in `folder` (the top level for `None`), in name order: what a tree
    /// row shows under it.
    pub fn children(&self, folder: Option<&str>) -> (Vec<&Folder>, Vec<&Session>) {
        let mut f: Vec<&Folder> = self.folders.iter().filter(|x| x.parent.as_deref() == folder).collect();
        let mut s: Vec<&Session> = self.sessions.iter().filter(|x| x.folder.as_deref() == folder).collect();
        f.sort_by_key(|x| x.name.to_lowercase());
        s.sort_by(|a, b| (a.id != DEFAULT_SESSION).cmp(&(b.id != DEFAULT_SESSION)).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        (f, s)
    }

    /// The tree, depth first, as (depth, row) for drawing: a collapsed folder's insides are left out.
    pub fn rows(&self) -> Vec<(usize, Row)> {
        let mut out = Vec::new();
        self.walk(None, 0, &mut out);
        out
    }

    fn walk(&self, folder: Option<&str>, depth: usize, out: &mut Vec<(usize, Row)>) {
        if depth > 64 {
            return;
        }
        let (folders, sessions) = self.children(folder);
        for f in folders {
            out.push((depth, Row::Folder(f.id.clone())));
            if !f.collapsed {
                self.walk(Some(&f.id), depth + 1, out);
            }
        }
        for s in sessions {
            out.push((depth, Row::Session(s.id.clone())));
        }
    }

    /// The folders, depth first with their depth, for a "move to" menu (collapsed ones too).
    pub fn folder_paths(&self) -> Vec<(usize, &Folder)> {
        fn go<'a>(s: &'a Sessions, parent: Option<&str>, d: usize, out: &mut Vec<(usize, &'a Folder)>) {
            if d > 64 {
                return;
            }
            for f in s.children(parent).0 {
                out.push((d, f));
                go(s, Some(&f.id), d + 1, out);
            }
        }
        let mut out = Vec::new();
        go(self, None, 0, &mut out);
        out
    }

    /// Whether `folder` is `ancestor` or inside it.
    fn within(&self, folder: &str, ancestor: &str) -> bool {
        let mut at = Some(folder.to_string());
        let mut hops = 0;
        while let Some(f) = at {
            if f == ancestor {
                return true;
            }
            hops += 1;
            if hops > 256 {
                return true;
            }
            at = self.folder(&f).and_then(|x| x.parent.clone());
        }
        false
    }

    fn check_folder(&self, folder: Option<&str>) -> Result<()> {
        match folder {
            Some(f) if self.folder(f).is_none() => Err(bad("no such folder")),
            _ => Ok(()),
        }
    }

    /// Repairs what a hand-edited or older file could hold: the default session, known folders.
    fn repair(&mut self) {
        if self.get(DEFAULT_SESSION).is_none() {
            self.sessions.insert(0, default_session());
        }
        if let Some(d) = self.sessions.iter_mut().find(|s| s.id == DEFAULT_SESSION) {
            d.target = Target::Weirdnet;
        }
        let ids: Vec<String> = self.folders.iter().map(|f| f.id.clone()).collect();
        for f in &mut self.folders {
            if f.parent.as_ref().is_some_and(|p| !ids.contains(p) || *p == f.id) {
                f.parent = None;
            }
        }
        // A cycle (a inside b inside a) is cut at the first folder found in it.
        for i in 0..self.folders.len() {
            let id = self.folders[i].id.clone();
            if let Some(p) = self.folders[i].parent.clone()
                && self.within(&p, &id)
            {
                self.folders[i].parent = None;
            }
        }
        for s in &mut self.sessions {
            if s.folder.as_ref().is_some_and(|f| !ids.contains(f)) {
                s.folder = None;
            }
        }
        if self.get(&self.active).is_none() {
            self.active = default_id();
        }
    }
}

/// A row of the drawn tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Folder(String),
    Session(String),
}

/// [`Sessions`] in a data folder.
pub(crate) struct SessionStore {
    path: PathBuf,
    pub(crate) data: Sessions,
}

impl SessionStore {
    pub(crate) fn open(dir: &Path) -> Self {
        let path = dir.join(SESSIONS_FILE);
        let mut data: Sessions = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        data.repair();
        Self { path, data }
    }

    fn save(&self) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.data).map_err(|e| CoreError::new(ErrorKind::Storage, e.to_string()))?;
        write_file(&self.path, &bytes, false)
    }

    /// Runs `f` on a copy and keeps it only when it succeeds and is saved.
    pub(crate) fn change<T>(&mut self, f: impl FnOnce(&mut Sessions) -> Result<T>) -> Result<T> {
        let mut next = self.data.clone();
        let out = f(&mut next)?;
        next.repair();
        let before = std::mem::replace(&mut self.data, next);
        if let Err(e) = self.save() {
            self.data = before;
            return Err(e);
        }
        Ok(out)
    }
}

// ---- the edits (on a copy, through `SessionStore::change`) ---------------------------------------

impl Sessions {
    pub(crate) fn create_session(
        &mut self,
        id: String,
        name: &str,
        target: Target,
        model: Option<String>,
        folder: Option<String>,
    ) -> Result<Session> {
        self.check_folder(folder.as_deref())?;
        let s =
            Session { id, name: clean_name(name, "session")?, target, model: model.filter(|m| !m.trim().is_empty()), folder, created_ms: now_ms() };
        self.sessions.push(s.clone());
        Ok(s)
    }

    fn session_mut(&mut self, id: &str) -> Result<&mut Session> {
        self.sessions.iter_mut().find(|s| s.id == id).ok_or_else(|| bad("no such session"))
    }

    pub(crate) fn rename_session(&mut self, id: &str, name: &str) -> Result<()> {
        let n = clean_name(name, "session")?;
        self.session_mut(id)?.name = n;
        Ok(())
    }

    pub(crate) fn move_session(&mut self, id: &str, folder: Option<String>) -> Result<()> {
        self.check_folder(folder.as_deref())?;
        self.session_mut(id)?.folder = folder;
        Ok(())
    }

    pub(crate) fn set_model(&mut self, id: &str, model: Option<String>) -> Result<()> {
        self.session_mut(id)?.model = model.filter(|m| !m.trim().is_empty());
        Ok(())
    }

    pub(crate) fn set_target(&mut self, id: &str, target: Target) -> Result<()> {
        if id == DEFAULT_SESSION && target != Target::Weirdnet {
            return Err(bad("the Weirdnet session always sends to your Weirdnet"));
        }
        let s = self.session_mut(id)?;
        if s.target != target {
            s.model = None;
        }
        s.target = target;
        Ok(())
    }

    pub(crate) fn switch(&mut self, id: &str) -> Result<()> {
        if self.get(id).is_none() {
            return Err(bad("no such session"));
        }
        self.active = id.to_string();
        Ok(())
    }

    /// Removes a session (not the default one); its conversations belong to the default session
    /// from then on, so nothing is lost.
    pub(crate) fn delete_session(&mut self, id: &str) -> Result<()> {
        if id == DEFAULT_SESSION {
            return Err(bad("the Weirdnet session can't be deleted"));
        }
        let before = self.sessions.len();
        self.sessions.retain(|s| s.id != id);
        if self.sessions.len() == before {
            return Err(bad("no such session"));
        }
        if self.active == id {
            self.active = default_id();
        }
        Ok(())
    }

    pub(crate) fn create_folder(&mut self, id: String, name: &str, parent: Option<String>) -> Result<Folder> {
        self.check_folder(parent.as_deref())?;
        let f = Folder { id, name: clean_name(name, "folder")?, parent, collapsed: false };
        self.folders.push(f.clone());
        Ok(f)
    }

    fn folder_mut(&mut self, id: &str) -> Result<&mut Folder> {
        self.folders.iter_mut().find(|f| f.id == id).ok_or_else(|| bad("no such folder"))
    }

    pub(crate) fn rename_folder(&mut self, id: &str, name: &str) -> Result<()> {
        let n = clean_name(name, "folder")?;
        self.folder_mut(id)?.name = n;
        Ok(())
    }

    /// Moves a folder (with everything in it) into `parent`; not into itself or its own subfolders.
    pub(crate) fn move_folder(&mut self, id: &str, parent: Option<String>) -> Result<()> {
        self.check_folder(parent.as_deref())?;
        if let Some(p) = &parent
            && self.within(p, id)
        {
            return Err(bad("a folder can't go inside itself"));
        }
        self.folder_mut(id)?.parent = parent;
        Ok(())
    }

    pub(crate) fn set_collapsed(&mut self, id: &str, collapsed: bool) -> Result<()> {
        self.folder_mut(id)?.collapsed = collapsed;
        Ok(())
    }

    /// Removes a folder; its sessions and subfolders move up to its parent (the shells ask first).
    pub(crate) fn delete_folder(&mut self, id: &str) -> Result<()> {
        let parent = self.folder(id).ok_or_else(|| bad("no such folder"))?.parent.clone();
        for f in &mut self.folders {
            if f.parent.as_deref() == Some(id) {
                f.parent = parent.clone();
            }
        }
        for s in &mut self.sessions {
            if s.folder.as_deref() == Some(id) {
                s.folder = parent.clone();
            }
        }
        self.folders.retain(|f| f.id != id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(dir: &Path) -> SessionStore {
        SessionStore::open(dir)
    }

    #[test]
    fn a_new_device_has_the_weirdnet_session_and_it_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = store(tmp.path());
        assert_eq!(s.data.active().id, DEFAULT_SESSION);
        assert_eq!(s.data.active().target, Target::Weirdnet);
        assert!(s.change(|d| d.delete_session(DEFAULT_SESSION)).is_err());
        assert!(s.change(|d| d.set_target(DEFAULT_SESSION, Target::Endpoint { id: "e".into() })).is_err());
        s.change(|d| d.rename_session(DEFAULT_SESSION, "My cloud")).unwrap();
        assert_eq!(store(tmp.path()).data.active().name, "My cloud");
    }

    #[test]
    fn sessions_persist_switch_and_their_conversations_fall_back_to_the_default() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = store(tmp.path());
        let a = s.change(|d| d.create_session("s-a".into(), " Work ", Target::Endpoint { id: "e-1".into() }, Some("m".into()), None)).unwrap();
        assert_eq!(a.name, "Work");
        s.change(|d| d.switch("s-a")).unwrap();
        let again = store(tmp.path());
        assert_eq!(again.data.active().id, "s-a");
        assert_eq!(again.data.owner(Some("s-a")), "s-a");
        assert_eq!(again.data.owner(Some("s-from-another-device")), DEFAULT_SESSION);
        assert_eq!(again.data.owner(None), DEFAULT_SESSION, "history from before sessions is the Weirdnet session's");
        s.change(|d| d.delete_session("s-a")).unwrap();
        assert_eq!(s.data.active().id, DEFAULT_SESSION);
        assert_eq!(s.data.owner(Some("s-a")), DEFAULT_SESSION, "a deleted session's conversations are kept, in the default session");
        assert!(s.change(|d| d.create_session("s-b".into(), "  ", Target::Weirdnet, None, None)).is_err(), "a name is needed");
    }

    #[test]
    fn folders_nest_move_collapse_and_hand_their_contents_up_when_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = store(tmp.path());
        s.change(|d| d.create_folder("f-top".into(), "Projects", None)).unwrap();
        s.change(|d| d.create_folder("f-sub".into(), "Bragi", Some("f-top".into()))).unwrap();
        s.change(|d| d.create_folder("f-deep".into(), "Ideas", Some("f-sub".into()))).unwrap();
        s.change(|d| d.create_session("s-1".into(), "Prompts", Target::Weirdnet, None, Some("f-sub".into()))).unwrap();
        assert!(s.change(|d| d.create_session("s-x".into(), "X", Target::Weirdnet, None, Some("f-none".into()))).is_err());

        let rows = s.data.rows();
        assert_eq!(
            rows,
            vec![
                (0, Row::Folder("f-top".into())),
                (1, Row::Folder("f-sub".into())),
                (2, Row::Folder("f-deep".into())),
                (2, Row::Session("s-1".into())),
                (0, Row::Session(DEFAULT_SESSION.into())),
            ]
        );
        s.change(|d| d.set_collapsed("f-sub", true)).unwrap();
        assert_eq!(s.data.rows().len(), 3, "a collapsed folder hides what is in it");
        assert!(store(tmp.path()).data.folder("f-sub").unwrap().collapsed, "collapsing persists");

        assert!(s.change(|d| d.move_folder("f-top", Some("f-deep".into()))).is_err(), "not into its own subfolder");
        assert!(s.change(|d| d.move_folder("f-top", Some("f-top".into()))).is_err(), "not into itself");
        s.change(|d| d.move_folder("f-deep", None)).unwrap();
        s.change(|d| d.move_session("s-1", Some("f-deep".into()))).unwrap();
        s.change(|d| d.move_session(DEFAULT_SESSION, Some("f-top".into()))).unwrap();
        s.change(|d| d.rename_folder("f-deep", "Later")).unwrap();

        // Deleting Projects: Bragi and the Weirdnet session move up to the top level.
        s.change(|d| d.delete_folder("f-top")).unwrap();
        let d = &store(tmp.path()).data;
        assert!(d.folder("f-top").is_none());
        assert_eq!(d.folder("f-sub").unwrap().parent, None);
        assert_eq!(d.get(DEFAULT_SESSION).unwrap().folder, None);
        assert_eq!(d.get("s-1").unwrap().folder.as_deref(), Some("f-deep"));
        assert_eq!(d.folder("f-deep").unwrap().name, "Later");
    }

    #[test]
    fn a_broken_file_is_repaired_not_lost() {
        let tmp = tempfile::tempdir().unwrap();
        let raw = r#"{"folders":[{"id":"a","name":"A","parent":"b"},{"id":"b","name":"B","parent":"a"}],
            "sessions":[{"id":"s","name":"S","target":{"kind":"weirdnet"},"folder":"gone","created_ms":1}],"active":"nope"}"#;
        std::fs::write(tmp.path().join(SESSIONS_FILE), raw).unwrap();
        let s = store(tmp.path());
        assert!(s.data.get(DEFAULT_SESSION).is_some());
        assert_eq!(s.data.active, DEFAULT_SESSION);
        assert_eq!(s.data.get("s").unwrap().folder, None);
        assert!(s.data.folders.iter().any(|f| f.parent.is_none()), "the cycle is cut");
        assert_eq!(s.data.rows().len(), 4);
    }
}

// ---- the core's session actions ------------------------------------------------------------------

impl crate::Core {
    /// The sessions, their folders and the open one.
    pub fn sessions(&self) -> Sessions {
        crate::core::lock(&self.inner.sessions).data.clone()
    }

    pub(crate) fn publish_sessions(&self) {
        let data = self.sessions();
        self.inner.state.send_if_modified(|s| {
            let changed = s.sessions != data;
            if changed {
                s.sessions = data;
            }
            changed
        });
    }

    fn edit_sessions<T>(&self, f: impl FnOnce(&mut Sessions) -> Result<T>) -> Result<T> {
        let out = crate::core::lock(&self.inner.sessions).change(f)?;
        self.publish_sessions();
        // An endpoint shows how many sessions send to it.
        self.publish_endpoints();
        Ok(out)
    }

    fn check_target(&self, target: &Target) -> Result<()> {
        if let Target::Endpoint { id } = target
            && crate::core::lock(&self.inner.endpoints).get(id).is_none()
        {
            return Err(bad("no such endpoint; add it under Endpoints first"));
        }
        Ok(())
    }

    /// A new session sending to `target` with `model` (the target's first when `None`), in
    /// `folder` (the top level when `None`). It doesn't open; [`switch_session`](Self::switch_session) does.
    pub fn create_session(&self, name: &str, target: Target, model: Option<String>, folder: Option<String>) -> Result<Session> {
        self.check_target(&target)?;
        let id = crate::core::random_id("s");
        self.edit_sessions(|d| d.create_session(id, name, target, model, folder))
    }

    pub fn rename_session(&self, id: &str, name: &str) -> Result<()> {
        self.edit_sessions(|d| d.rename_session(id, name))
    }

    /// Puts a session in `folder` (the top level when `None`).
    pub fn move_session(&self, id: &str, folder: Option<String>) -> Result<()> {
        self.edit_sessions(|d| d.move_session(id, folder))
    }

    /// The model a session asks for (its target's first when `None`).
    pub fn set_session_model(&self, id: &str, model: Option<String>) -> Result<()> {
        self.edit_sessions(|d| d.set_model(id, model))
    }

    /// Where a session sends (not for the Weirdnet session). Its model is reset.
    pub fn set_session_target(&self, id: &str, target: Target) -> Result<()> {
        self.check_target(&target)?;
        self.edit_sessions(|d| d.set_target(id, target))
    }

    /// Opens a session: new conversations go to its target and the list shows its history.
    pub fn switch_session(&self, id: &str) -> Result<()> {
        self.edit_sessions(|d| d.switch(id))
    }

    /// Deletes a session (not the Weirdnet one). Its conversations aren't deleted: they show in
    /// the Weirdnet session from then on.
    pub fn delete_session(&self, id: &str) -> Result<()> {
        self.edit_sessions(|d| d.delete_session(id))
    }

    /// A new folder in `parent` (the top level when `None`).
    pub fn create_folder(&self, name: &str, parent: Option<String>) -> Result<Folder> {
        let id = crate::core::random_id("f");
        self.edit_sessions(|d| d.create_folder(id, name, parent))
    }

    pub fn rename_folder(&self, id: &str, name: &str) -> Result<()> {
        self.edit_sessions(|d| d.rename_folder(id, name))
    }

    /// Moves a folder and everything in it into `parent` (the top level when `None`).
    pub fn move_folder(&self, id: &str, parent: Option<String>) -> Result<()> {
        self.edit_sessions(|d| d.move_folder(id, parent))
    }

    pub fn set_folder_collapsed(&self, id: &str, collapsed: bool) -> Result<()> {
        self.edit_sessions(|d| d.set_collapsed(id, collapsed))
    }

    /// Deletes a folder; what was in it moves up to its parent. Shells confirm first.
    pub fn delete_folder(&self, id: &str) -> Result<()> {
        self.edit_sessions(|d| d.delete_folder(id))
    }
}
