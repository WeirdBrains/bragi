//! History (B4): conversations live on the shared context store ([`crate::context`]), so they
//! follow the user between devices. B1's local files (`<dir>/history/*.json`) are imported the
//! first time a linked device opens the store. The shapes here are what the shells draw.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};

use crate::context::{ContextStore, Stored, StoredConversation, updated_ms};
use crate::error::{CoreError, ErrorKind, Result};
use crate::identity::Identity;

mod forget;
pub use forget::{Deleted, UNDO_MS};

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
    /// The model's reasoning ("thinking") for an assistant answer, shown apart from it and never
    /// sent back to the model. Empty when the model didn't reason.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reasoning: String,
    /// How long the model reasoned before answering, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_ms: Option<u64>,
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
    /// Emoji the user reacted with, in the order they were added (no repeats). Absent in
    /// records written before reactions existed. They ride along in the stored record, so
    /// the user's other devices and a Ymir pool reading the context see them too.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<String>,
    /// The mode the message was written in: a chat (or plan) turn, or a code turn (files as
    /// context, edits reviewed as diffs, shell runs). One conversation holds both; records
    /// written before modes existed are chat.
    #[serde(default, skip_serializing_if = "Mode::is_chat")]
    pub mode: Mode,
    pub at_ms: i64,
}

/// Chat or code: how a turn was made. Switching keeps the conversation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Chat,
    Code,
}

impl Mode {
    pub fn is_chat(&self) -> bool {
        *self == Mode::Chat
    }

    pub fn other(self) -> Self {
        match self {
            Mode::Chat => Mode::Code,
            Mode::Code => Mode::Chat,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Chat => "chat",
            Mode::Code => "code",
        }
    }
}

impl ChatMessage {
    /// A whole message written now (no id until saved).
    pub fn new(role: Role, text: impl Into<String>, mode: Mode) -> Self {
        Self {
            id: String::new(),
            role,
            text: text.into(),
            reasoning: String::new(),
            thinking_ms: None,
            state: MessageState::Complete,
            model: None,
            node: None,
            cache_hit: false,
            standin: None,
            resumes: 0,
            attachments: Vec::new(),
            tools: Vec::new(),
            agent: false,
            reactions: Vec::new(),
            mode,
            at_ms: now_ms(),
        }
    }
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
    /// The user marked it for training (its own record, see [`crate::context::Train`]); off
    /// unless set. Not written by [`History::save`]: [`History::set_training`] sets it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub training: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub updated_ms: i64,
    pub messages: usize,
}

impl Conversation {
    pub fn summary(&self) -> ConversationSummary {
        ConversationSummary { id: self.id.clone(), title: self.title.clone(), updated_ms: self.updated_ms, messages: self.messages.len() }
    }

    /// The answer that followed user message `id`, if the next message is one: deleting a
    /// question offers to delete it too.
    pub fn reply_to(&self, id: &str) -> Option<&ChatMessage> {
        let i = self.messages.iter().position(|m| m.id == id)?;
        let next = self.messages.get(i + 1)?;
        (self.messages[i].role == Role::User && next.role == Role::Assistant).then_some(next)
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
    dir: Mutex<PathBuf>,
    store: RwLock<Option<Arc<ContextStore>>>,
    /// Ids handed to messages this `History` saved; none is reused.
    seq: Mutex<u16>,
    /// What the last delete removed, for [`History::undo`] (see [`forget`]).
    undo: Mutex<Option<forget::Undo>>,
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
    if !m.reasoning.is_empty() {
        ext.insert("reasoning".into(), m.reasoning.clone().into());
    }
    if let Some(ms) = m.thinking_ms {
        ext.insert("thinking_ms".into(), ms.into());
    }
    if !m.reactions.is_empty() {
        ext.insert("reactions".into(), serde_json::to_value(&m.reactions).unwrap_or_default());
    }
    if !m.mode.is_chat() {
        ext.insert("mode".into(), m.mode.name().into());
    }
    Stored { id, role: role_str(m.role).into(), text: m.text.clone(), model: m.model.clone(), at_ms: m.at_ms, ext }
}

fn from_stored(m: &Stored) -> ChatMessage {
    let get = |k: &str| m.ext.get(k);
    let assistant = m.role == "assistant";
    let mut reasoning = get("reasoning").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let mut text = m.text.clone();
    // An answer saved with its `<think>` block still in the text (by an app that doesn't split it).
    if assistant && reasoning.is_empty() && text.trim_start().starts_with("<think>") {
        (reasoning, text) = crate::think::split(&text);
    }
    ChatMessage {
        id: m.id.clone(),
        role: if assistant { Role::Assistant } else { Role::User },
        text,
        reasoning,
        thinking_ms: get("thinking_ms").and_then(|v| v.as_u64()),
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
        reactions: get("reactions").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default(),
        mode: get("mode").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default(),
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
        training: c.train.on,
    }
}

impl History {
    /// `dir` is the app's data directory: B1's files, if any, are in `<dir>/history/`.
    pub fn open(dir: &Path) -> Result<Self> {
        Ok(Self { dir: Mutex::new(dir.join("history")), store: RwLock::new(None), seq: Mutex::new(0), undo: Mutex::new(None) })
    }

    fn history_dir(&self) -> PathBuf {
        self.dir.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Drops the context store so a different identity can be attached.
    pub fn detach(&self) {
        *self.store.write().unwrap_or_else(|e| e.into_inner()) = None;
        *self.undo.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self.seq.lock().unwrap_or_else(|e| e.into_inner()) = 0;
    }

    /// Points history at another pool's directory. The store is cleared until the next [`attach`].
    pub fn switch_root(&self, dir: &Path) {
        self.detach();
        *self.dir.lock().unwrap_or_else(|e| e.into_inner()) = dir.join("history");
    }

    /// Puts history on `identity`'s context store (the device is linked), importing B1's local
    /// files once. Returns how many conversations were imported.
    pub fn attach(&self, identity: Arc<Identity>) -> usize {
        let store = Arc::new(ContextStore::new(identity));
        *self.store.write().unwrap_or_else(|e| e.into_inner()) = Some(store);
        let n = self.import_local();
        // Deletes made before deleting compacted (or on another device) leave nothing behind.
        self.compact();
        n
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
        let dir = self.history_dir();
        let Ok(rd) = std::fs::read_dir(&dir) else { return 0 };
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
        let aside = dir.with_file_name("history.imported");
        if let Err(e) = std::fs::rename(&dir, &aside) {
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

    /// Marks a conversation for training, or withdraws it. It has to exist.
    pub fn set_training(&self, id: &str, on: bool) -> Result<()> {
        self.load(id)?;
        self.store()?.set_train(id, on).map_err(storage)
    }

    /// Deletes one conversation (see [`History::delete_conversations`]).
    pub fn delete(&self, id: &str) -> Result<()> {
        self.delete_conversations(&[id.to_string()]).map(|_| ())
    }

    /// Newest first: `offset` and `limit` page through them.
    pub fn page(&self, offset: usize, limit: usize) -> Result<Vec<ConversationSummary>> {
        Ok(self.store()?.list().iter().map(|c| from_conversation(c).summary()).skip(offset).take(limit).collect())
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

    pub(super) fn linked(dir: &Path) -> (Arc<Identity>, History) {
        let id = Arc::new(Identity::create(&dir.join("identity"), "t").unwrap());
        let h = History::open(dir).unwrap();
        h.attach(id.clone());
        (id, h)
    }

    pub(super) fn conv(id: &str, texts: &[&str]) -> Conversation {
        let messages = texts
            .iter()
            .enumerate()
            .map(|(i, t)| ChatMessage {
                id: String::new(),
                role: if i % 2 == 0 { Role::User } else { Role::Assistant },
                text: t.to_string(),
                reasoning: String::new(),
                thinking_ms: None,
                state: MessageState::Complete,
                model: (i % 2 == 1).then(|| "alpha".to_string()),
                node: None,
                cache_hit: false,
                standin: None,
                resumes: 0,
                attachments: Vec::new(),
                tools: Vec::new(),
                agent: false,
                reactions: Vec::new(),
                mode: Default::default(),
                // The same millisecond for all of them: the order must still hold.
                at_ms: 1000,
            })
            .collect();
        Conversation { id: id.into(), title: title_for(texts[0]), created_ms: 1000, updated_ms: 1000, messages, training: false }
    }

    #[test]
    fn training_consent_is_off_until_set_survives_a_save_and_is_ymirs_record() {
        let dir = tempfile::tempdir().unwrap();
        let (id, h) = linked(dir.path());
        h.save(&conv("c1", &["q", "a"])).unwrap();
        assert!(!h.load("c1").unwrap().training);
        assert!(h.set_training("nope", true).is_err());
        h.set_training("c1", true).unwrap();
        // Saving the conversation again (a new title, a new message) keeps the consent.
        let mut c = h.load("c1").unwrap();
        assert!(c.training);
        c.title = "Renamed".into();
        h.save(&c).unwrap();
        assert!(h.load("c1").unwrap().training);
        // Ymir's layout: `{conversation}/train` with `on`.
        let recs = id.records(crate::identity::CONTEXT_TYPE);
        let (_, v) = recs.iter().find(|(k, _)| k == "c1/train").expect("the train record");
        let t: serde_json::Value = serde_json::from_slice(v).unwrap();
        assert_eq!(t["on"], true);
        h.set_training("c1", false).unwrap();
        assert!(!h.load("c1").unwrap().training);
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

    #[test]
    fn reasoning_is_kept_and_inline_blocks_are_split() {
        let mut m = conv("c", &["q", "391"]).messages.remove(1);
        m.reasoning = "17*20 + 17*3".into();
        m.thinking_ms = Some(1500);
        let back = from_stored(&to_stored(&m, "m1".into()));
        assert_eq!((back.reasoning.as_str(), back.thinking_ms, back.text.as_str()), ("17*20 + 17*3", Some(1500), "391"));
        // Saved by an app that left the block in the text.
        let mut raw = to_stored(&conv("c", &["q", "x"]).messages[1], "m2".into());
        raw.text = "<think>hm</think>\n\n391".into();
        let back = from_stored(&raw);
        assert_eq!((back.reasoning.as_str(), back.text.as_str()), ("hm", "391"));
    }
}

#[cfg(test)]
mod reaction_tests {
    use super::*;

    #[test]
    fn reactions_round_trip_and_old_records_load() {
        let dir = tempfile::tempdir().unwrap();
        let id = Arc::new(Identity::create(&dir.path().join("identity"), "t").unwrap());
        let h = History::open(dir.path()).unwrap();
        h.attach(id);
        let m = |role, text: &str| ChatMessage {
            id: String::new(),
            role,
            text: text.into(),
            reasoning: String::new(),
            thinking_ms: None,
            state: MessageState::Complete,
            model: None,
            node: None,
            cache_hit: false,
            standin: None,
            resumes: 0,
            attachments: Vec::new(),
            tools: Vec::new(),
            agent: false,
            reactions: Vec::new(),
            mode: Default::default(),
            at_ms: 5,
        };
        let mut c = Conversation {
            id: "c1".into(),
            title: "t".into(),
            created_ms: 5,
            updated_ms: 5,
            messages: vec![m(Role::User, "a"), m(Role::Assistant, "b")],
            training: false,
        };
        h.save(&c).unwrap();
        // A record written before reactions existed has none.
        assert!(h.load("c1").unwrap().messages.iter().all(|m| m.reactions.is_empty()));
        c = h.load("c1").unwrap();
        c.messages[1].reactions = vec!["👍".into(), "❤️".into()];
        h.save(&c).unwrap();
        assert_eq!(h.load("c1").unwrap().messages[1].reactions, ["👍", "❤️"]);
        // Old JSON without the field still parses.
        let j = r#"{"role":"user","text":"x","state":"complete","at_ms":1}"#;
        assert!(serde_json::from_str::<ChatMessage>(j).unwrap().reactions.is_empty());
    }

    #[test]
    fn a_message_keeps_its_mode_in_the_store_and_old_ones_are_chat() {
        let m = ChatMessage::new(Role::User, "make it", Mode::Code);
        let stored = to_stored(&m, "x".into());
        assert_eq!(stored.ext.get("mode").and_then(|v| v.as_str()), Some("code"));
        assert_eq!(from_stored(&stored).mode, Mode::Code);
        let chat = to_stored(&ChatMessage::new(Role::User, "hi", Mode::Chat), "y".into());
        assert!(!chat.ext.contains_key("mode"), "chat is the default and isn't written");
        assert_eq!(from_stored(&chat).mode, Mode::Chat);
        // JSON from before modes.
        let old: ChatMessage = serde_json::from_str(r#"{"role":"user","text":"hi","state":"complete","at_ms":1}"#).unwrap();
        assert_eq!(old.mode, Mode::Chat);
    }
}
