//! History (B4): conversations live on the shared context store ([`crate::context`]), so they
//! follow the user between devices. B1's local files (`<dir>/history/*.json`) are imported the
//! first time a linked device opens the store. The shapes here are what the shells draw.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};

use crate::context::{ContextStore, Stored, StoredConversation, updated_ms};
use crate::error::{CoreError, ErrorKind, Result};
use crate::identity::Identity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

/// How an assistant message ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageState {
    Complete,
    /// Still arriving.
    Streaming,
    /// Stopped by the user; it can be continued.
    Stopped,
    /// Cut off by an error; it can be continued.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// The record's id in the store. Empty on a message not saved yet.
    #[serde(default)]
    pub id: String,
    pub role: Role,
    pub text: String,
    pub state: MessageState,
    /// The model that answered (assistant messages).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The inference node that answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// Served from the pool's answer cache.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cache_hit: bool,
    /// The model that answered while the requested one loaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standin: Option<String>,
    /// Times the answer moved to another node mid-stream.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub resumes: u32,
    /// Text files attached to a user message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<crate::attach::Attachment>,
    /// Agent mode: the tools the task used, in order (assistant messages).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// The answer came from an agent task on a node, not a plain model reply.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub agent: bool,
    /// The endpoint that answered (its name) when it wasn't the Weirdnet pool: a session's own
    /// endpoint, or the fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    pub at_ms: i64,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub created_ms: i64,
    pub updated_ms: i64,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    /// The session it was started in (see [`crate::session`]); none for older history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub updated_ms: i64,
    pub messages: usize,
    #[serde(default)]
    pub session: Option<String>,
}

impl Conversation {
    pub fn summary(&self) -> ConversationSummary {
        ConversationSummary {
            id: self.id.clone(),
            title: self.title.clone(),
            updated_ms: self.updated_ms,
            messages: self.messages.len(),
            session: self.session.clone(),
        }
    }

    /// The last message, if it is an assistant answer that was cut off.
    pub fn continuable(&self) -> bool {
        self.messages.last().is_some_and(|m| m.role == Role::Assistant && matches!(m.state, MessageState::Stopped | MessageState::Failed))
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// A short title from the first prompt.
pub fn title_for(prompt: &str) -> String {
    let line = prompt.lines().next().unwrap_or("").trim();
    let mut t: String = line.chars().take(48).collect();
    if line.chars().count() > 48 {
        t.push('…');
    }
    if t.is_empty() { "New chat".into() } else { t }
}

/// Conversations by id, over the context store once the device is linked. Before that there is
/// no history: chatting needs a link anyway.
pub struct History {
    dir: PathBuf,
    store: RwLock<Option<Arc<ContextStore>>>,
    /// Ids handed to messages this `History` saved; none is reused.
    seq: Mutex<u16>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn storage(e: impl std::fmt::Display) -> CoreError {
    CoreError::new(ErrorKind::Storage, e.to_string())
}

fn not_found(id: &str) -> CoreError {
    CoreError::new(ErrorKind::NotFound, format!("no conversation `{id}`"))
}

fn role_str(r: Role) -> &'static str {
    match r {
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

fn to_stored(m: &ChatMessage, id: String) -> Stored {
    let mut ext = serde_json::Map::new();
    ext.insert("state".into(), serde_json::to_value(m.state).unwrap_or_default());
    if let Some(n) = &m.node {
        ext.insert("node".into(), n.clone().into());
    }
    if m.cache_hit {
        ext.insert("cache_hit".into(), true.into());
    }
    if let Some(s) = &m.standin {
        ext.insert("standin".into(), s.clone().into());
    }
    if m.resumes > 0 {
        ext.insert("resumes".into(), m.resumes.into());
    }
    if !m.attachments.is_empty() {
        ext.insert("attachments".into(), serde_json::to_value(&m.attachments).unwrap_or_default());
    }
    if !m.tools.is_empty() {
        ext.insert("tools".into(), serde_json::to_value(&m.tools).unwrap_or_default());
    }
    if m.agent {
        ext.insert("agent".into(), true.into());
    }
    if let Some(v) = &m.via {
        ext.insert("via".into(), v.clone().into());
    }
    Stored { id, role: role_str(m.role).into(), text: m.text.clone(), model: m.model.clone(), at_ms: m.at_ms, ext }
}

fn from_stored(m: &Stored) -> ChatMessage {
    let get = |k: &str| m.ext.get(k);
    ChatMessage {
        id: m.id.clone(),
        role: if m.role == "assistant" { Role::Assistant } else { Role::User },
        text: m.text.clone(),
        // Messages from apps that don't track it (Ymir's) are whole.
        state: get("state").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or(MessageState::Complete),
        model: m.model.clone(),
        node: get("node").and_then(|v| v.as_str()).map(str::to_string),
        cache_hit: get("cache_hit").and_then(|v| v.as_bool()).unwrap_or(false),
        standin: get("standin").and_then(|v| v.as_str()).map(str::to_string),
        resumes: get("resumes").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        attachments: get("attachments").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default(),
        tools: get("tools").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default(),
        agent: get("agent").and_then(|v| v.as_bool()).unwrap_or(false),
        via: get("via").and_then(|v| v.as_str()).map(str::to_string),
        at_ms: m.at_ms,
    }
}

fn from_conversation(c: &StoredConversation) -> Conversation {
    let created = c.meta.ext.get("created_ms").and_then(|v| v.as_i64()).unwrap_or_else(|| c.messages.first().map_or(c.meta.at_ms, |m| m.at_ms));
    Conversation {
        id: c.id.clone(),
        title: if c.meta.title.is_empty() { "New chat".into() } else { c.meta.title.clone() },
        created_ms: created,
        updated_ms: updated_ms(c),
        messages: c.messages.iter().map(from_stored).collect(),
        session: c.meta.ext.get("session").and_then(|v| v.as_str()).map(str::to_string),
    }
}

impl History {
    /// `dir` is the app's data directory: B1's files, if any, are in `<dir>/history/`.
    pub fn open(dir: &Path) -> Result<Self> {
        Ok(Self { dir: dir.join("history"), store: RwLock::new(None), seq: Mutex::new(0) })
    }

    /// Puts history on `identity`'s context store (the device is linked), importing B1's local
    /// files once. Returns how many conversations were imported.
    pub fn attach(&self, identity: Arc<Identity>) -> usize {
        let store = Arc::new(ContextStore::new(identity));
        *self.store.write().unwrap_or_else(|e| e.into_inner()) = Some(store);
        self.import_local()
    }

    fn store(&self) -> Result<Arc<ContextStore>> {
        self.store
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| CoreError::new(ErrorKind::NotPaired, "link this device to your Weirdnet first"))
    }

    /// Reads B1's per-conversation files into the store and moves the directory aside.
    fn import_local(&self) -> usize {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return 0 };
        let mut n = 0;
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "json")
                && let Ok(b) = std::fs::read(&p)
                && let Ok(c) = serde_json::from_slice::<Conversation>(&b)
                && valid_id(&c.id)
            {
                match self.save(&c) {
                    Ok(()) => n += 1,
                    Err(e) => tracing::warn!("importing {}: {e}", c.id),
                }
            }
        }
        let aside = self.dir.with_file_name("history.imported");
        if let Err(e) = std::fs::rename(&self.dir, &aside) {
            tracing::warn!("moving the imported history aside: {e}");
        }
        n
    }

    pub fn load(&self, id: &str) -> Result<Conversation> {
        if !valid_id(id) {
            return Err(not_found(id));
        }
        self.store()?.load(id).map(|c| from_conversation(&c)).ok_or_else(|| not_found(id))
    }

    /// Writes what changed: the title and times, and every message whose record differs. A
    /// message without an id gets one.
    pub fn save(&self, c: &Conversation) -> Result<()> {
        if !valid_id(&c.id) {
            return Err(not_found(&c.id));
        }
        let store = self.store()?;
        let current = store.load(&c.id);
        let mut meta = current.as_ref().map(|s| s.meta.clone()).unwrap_or_default();
        let before = meta.clone();
        meta.title = c.title.clone();
        meta.deleted = false;
        meta.ext.insert("created_ms".into(), c.created_ms.into());
        meta.ext.insert("updated_ms".into(), c.updated_ms.into());
        match &c.session {
            Some(sid) => {
                meta.ext.insert("session".into(), sid.clone().into());
            }
            None => {
                meta.ext.remove("session");
            }
        }
        if current.is_none() || !before.same(&meta) {
            store.set_meta(&c.id, meta).map_err(storage)?;
        }
        for (i, m) in c.messages.iter().enumerate() {
            let id = if m.id.is_empty() { self.new_message_id(m.at_ms, i) } else { m.id.clone() };
            let stored = to_stored(m, id.clone());
            let same = current.as_ref().and_then(|s| s.messages.iter().find(|x| x.id == id)).is_some_and(|x| *x == stored);
            if !same {
                store.set_message(&c.id, &stored).map_err(storage)?;
            }
        }
        Ok(())
    }

    /// Ids sort by time, then position in the conversation, so two messages written in the same
    /// millisecond keep their order.
    fn new_message_id(&self, at_ms: i64, index: usize) -> String {
        let mut seq = self.seq.lock().unwrap_or_else(|e| e.into_inner());
        *seq = seq.wrapping_add(1);
        let mut b = [0u8; 2];
        let _ = getrandom::fill(&mut b);
        format!("{at_ms:013}-{index:04}-{:04x}{:04x}", u16::from_le_bytes(b), *seq)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        if !valid_id(id) {
            return Err(not_found(id));
        }
        let store = self.store()?;
        if store.load(id).is_none() {
            return Err(not_found(id));
        }
        store.delete(id).map_err(storage)
    }

    /// Newest first: `offset` and `limit` page through them.
    pub fn page(&self, offset: usize, limit: usize) -> Result<Vec<ConversationSummary>> {
        Ok(self.store()?.list().iter().map(|c| from_conversation(c).summary()).skip(offset).take(limit).collect())
    }

    /// [`page`](Self::page) over the conversations `keep` says yes to (one session's).
    pub fn page_where(&self, offset: usize, limit: usize, keep: impl Fn(&ConversationSummary) -> bool) -> Result<Vec<ConversationSummary>> {
        Ok(self.store()?.list().iter().map(|c| from_conversation(c).summary()).filter(|c| keep(c)).skip(offset).take(limit).collect())
    }

    /// Writes what was waiting for the first key (see [`ContextStore::flush`]).
    pub fn flush(&self) -> usize {
        self.store().map(|s| s.flush()).unwrap_or(0)
    }

    /// Changes whenever the stored conversations do, so a shell knows to redraw after a sync.
    pub fn fingerprint(&self) -> u64 {
        self.store().map(|s| s.fingerprint()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linked(dir: &Path) -> (Arc<Identity>, History) {
        let id = Arc::new(Identity::create(&dir.join("identity"), "t").unwrap());
        let h = History::open(dir).unwrap();
        h.attach(id.clone());
        (id, h)
    }

    fn conv(id: &str, texts: &[&str]) -> Conversation {
        let messages = texts
            .iter()
            .enumerate()
            .map(|(i, t)| ChatMessage {
                id: String::new(),
                role: if i % 2 == 0 { Role::User } else { Role::Assistant },
                text: t.to_string(),
                state: MessageState::Complete,
                model: (i % 2 == 1).then(|| "alpha".to_string()),
                node: None,
                cache_hit: false,
                standin: None,
                resumes: 0,
                attachments: Vec::new(),
                tools: Vec::new(),
                agent: false,
                via: None,
                // The same millisecond for all of them: the order must still hold.
                at_ms: 1000,
            })
            .collect();
        Conversation { id: id.into(), title: title_for(texts[0]), created_ms: 1000, updated_ms: 1000, messages, session: None }
    }

    #[test]
    fn save_page_load_delete_on_the_context_store() {
        let tmp = tempfile::tempdir().unwrap();
        let (_id, h) = linked(tmp.path());
        for (i, id) in ["a1", "b2", "c3"].iter().enumerate() {
            let mut c = conv(id, &["hi", "hello", "more"]);
            c.updated_ms = i as i64;
            h.save(&c).unwrap();
        }
        let page = h.page(0, 2).unwrap();
        assert_eq!(page.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["c3", "b2"]);
        assert_eq!(h.page(2, 5).unwrap().len(), 1);
        let b = h.load("b2").unwrap();
        assert_eq!(b.updated_ms, 1);
        assert_eq!(b.messages.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(), ["hi", "hello", "more"]);
        assert_eq!(h.load("../x").unwrap_err().kind, ErrorKind::NotFound);
        h.delete("a1").unwrap();
        assert_eq!(h.page(0, 10).unwrap().len(), 2);
        assert_eq!(h.load("a1").unwrap_err().kind, ErrorKind::NotFound);
        assert_eq!(title_for(&"x".repeat(60)).chars().count(), 49);
    }

    #[test]
    fn a_conversation_keeps_its_session_and_the_endpoint_that_answered() {
        let tmp = tempfile::tempdir().unwrap();
        let (_id, h) = linked(tmp.path());
        let mut c = conv("s1", &["hi", "hello"]);
        c.session = Some("s-work".into());
        c.messages[1].via = Some("Home bridge".into());
        h.save(&c).unwrap();
        h.save(&conv("old", &["before sessions"])).unwrap();
        let back = h.load("s1").unwrap();
        assert_eq!(back.session.as_deref(), Some("s-work"));
        assert_eq!(back.messages[1].via.as_deref(), Some("Home bridge"));
        assert_eq!(h.load("old").unwrap().session, None);
        let work = h.page_where(0, 10, |c| c.session.as_deref() == Some("s-work")).unwrap();
        assert_eq!(work.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["s1"]);
        // Moving it back to no session drops the key.
        c.session = None;
        h.save(&c).unwrap();
        assert_eq!(h.load("s1").unwrap().session, None);
    }

    #[test]
    fn a_streaming_answer_updates_in_place_and_survives_a_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let (id, h) = linked(tmp.path());
        h.save(&conv("c", &["q", ""])).unwrap();
        let mut c = h.load("c").unwrap();
        c.messages[1].state = MessageState::Streaming;
        c.messages[1].text = "par".into();
        h.save(&c).unwrap();
        let mut c = h.load("c").unwrap();
        assert_eq!(c.messages.len(), 2, "the same record, not a new one");
        c.messages[1].text = "partial answer".into();
        c.messages[1].state = MessageState::Stopped;
        c.messages[1].node = Some("n1".into());
        c.messages[1].cache_hit = true;
        h.save(&c).unwrap();
        drop((id, h));
        let (_id, h) = (Arc::new(Identity::open(&tmp.path().join("identity")).unwrap().unwrap()), History::open(tmp.path()).unwrap());
        h.attach(_id.clone());
        let c = h.load("c").unwrap();
        assert!(c.continuable());
        assert_eq!((c.messages[1].text.as_str(), c.messages[1].node.as_deref(), c.messages[1].cache_hit), ("partial answer", Some("n1"), true));
        // More writes carry on after the restart.
        let mut c = c;
        c.messages.push(ChatMessage { text: "again".into(), role: Role::User, at_ms: 2000, ..c.messages[0].clone() });
        c.messages.last_mut().unwrap().id.clear();
        h.save(&c).unwrap();
        assert_eq!(h.load("c").unwrap().messages.len(), 3);
    }

    #[test]
    fn b1_local_files_are_imported_once() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("history");
        std::fs::create_dir_all(&old).unwrap();
        // B1's shape: messages without ids.
        let mut c = conv("old-1", &["from before", "yes"]);
        c.updated_ms = 5;
        std::fs::write(old.join("old-1.json"), serde_json::to_vec(&c).unwrap()).unwrap();
        let (_id, h) = linked(tmp.path());
        let got = h.load("old-1").unwrap();
        assert_eq!((got.messages.len(), got.updated_ms, got.title.as_str()), (2, 5, "from before"));
        assert!(!old.exists() && tmp.path().join("history.imported").is_dir());
    }

    #[test]
    fn the_records_have_ymirs_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let (id, h) = linked(tmp.path());
        h.save(&conv("c1", &["hi", "hello"])).unwrap();
        let recs = id.records(crate::identity::CONTEXT_TYPE);
        let mut names: Vec<&str> = recs.iter().map(|(k, _)| k.as_str()).collect();
        names.sort();
        assert_eq!(names.iter().filter(|n| n.starts_with("c1/m/")).count(), 2);
        assert!(names.contains(&"c1/meta"));
        let (_, m) = recs.iter().find(|(k, _)| k.starts_with("c1/m/")).unwrap();
        let v: serde_json::Value = serde_json::from_slice(m).unwrap();
        // The fields Ymir reads.
        assert!(v["role"].is_string() && v["text"].is_string() && v["at_ms"].is_i64());
    }

    #[test]
    fn unlinked_has_no_history() {
        let tmp = tempfile::tempdir().unwrap();
        let h = History::open(tmp.path()).unwrap();
        assert_eq!(h.page(0, 5).unwrap_err().kind, ErrorKind::NotPaired);
        assert_eq!(h.load("x").unwrap_err().kind, ErrorKind::NotPaired);
    }
}
