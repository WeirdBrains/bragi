//! Conversations on the shared context store (B4): sealed `weirdnet-sync` records of type
//! `ymir.context`, one `{conversation}/meta` record and one `{conversation}/m/{id}` record per
//! message. The layout is Ymir's (`ymir-core`'s `context`), so a conversation started in Bragi
//! can be read by a Ymir pool and the other way round; Bragi's own fields ride along as extra
//! JSON keys, which Ymir ignores. Generic context code like this belongs in `weirdnet-agent`
//! later; until it is released it lives here, by copy.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::identity::{CONTEXT_TYPE, Identity, WriteError};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub deleted: bool,
    /// When this record was written.
    #[serde(default)]
    pub at_ms: i64,
    /// Fields other apps add (Bragi keeps `created_ms` and `updated_ms` here).
    #[serde(flatten)]
    pub ext: Map<String, Value>,
}

impl Meta {
    /// Whether `other` says the same thing (ignoring when it was written).
    pub fn same(&self, other: &Meta) -> bool {
        self.title == other.title
            && self.system == other.system
            && self.model == other.model
            && self.deleted == other.deleted
            && self.ext == other.ext
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stored {
    /// The record's id within the conversation; not part of the value.
    #[serde(skip)]
    pub id: String,
    pub role: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub at_ms: i64,
    /// Fields other apps add (Bragi keeps how an answer ended, its node and its flags here).
    #[serde(flatten)]
    pub ext: Map<String, Value>,
}

/// One conversation as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredConversation {
    pub id: String,
    pub meta: Meta,
    pub messages: Vec<Stored>,
}

/// The store over this device's [`Identity`]. A write that has to wait for the first key (a
/// device linked into a Weirdnet with others) is kept in memory, shows in reads, and goes out
/// on [`ContextStore::flush`] once a sync pass brought the key.
const PENDING: &str = "pending.json";

pub struct ContextStore {
    id: Arc<Identity>,
    pending: Mutex<BTreeMap<String, Vec<u8>>>,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl ContextStore {
    pub fn new(id: Arc<Identity>) -> Self {
        let waiting = std::fs::read(id.dir().join(PENDING)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { id, pending: Mutex::new(waiting) }
    }

    /// Keeps the waiting writes across a restart.
    fn save_pending(&self) {
        let p = self.pending();
        let path = self.id.dir().join(PENDING);
        let r = if p.is_empty() {
            std::fs::remove_file(&path).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
        } else {
            serde_json::to_vec(&*p).map_err(std::io::Error::other).and_then(|b| crate::identity::write_private_file(&path, &b))
        };
        if let Err(e) = r {
            tracing::warn!("keeping the waiting history writes: {e}");
        }
    }

    fn pending(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Vec<u8>>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn put(&self, record: &str, value: Vec<u8>) -> anyhow::Result<()> {
        // Older waiting writes go first, so the order of one conversation holds.
        self.flush();
        if self.pending().is_empty() {
            match self.id.write_record(CONTEXT_TYPE, record, &value) {
                Ok(()) => return Ok(()),
                Err(WriteError::NoKeyYet) => {}
                Err(WriteError::Other(e)) => return Err(e),
            }
        }
        self.pending().insert(record.to_string(), value);
        self.save_pending();
        Ok(())
    }

    /// Writes what was waiting for the first key. Returns how many are still waiting.
    pub fn flush(&self) -> usize {
        let waiting: Vec<(String, Vec<u8>)> = self.pending().iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (k, v) in waiting {
            match self.id.write_record(CONTEXT_TYPE, &k, &v) {
                Ok(()) => {
                    let mut p = self.pending();
                    if p.get(&k) == Some(&v) {
                        p.remove(&k);
                    }
                }
                Err(WriteError::NoKeyYet) => break,
                Err(WriteError::Other(e)) => {
                    tracing::warn!("writing {k}: {e}");
                    self.pending().remove(&k);
                }
            }
        }
        self.save_pending();
        self.pending().len()
    }

    pub fn set_meta(&self, conversation: &str, meta: Meta) -> anyhow::Result<()> {
        check_id(conversation)?;
        let meta = Meta { at_ms: now_ms(), ..meta };
        self.put(&format!("{conversation}/meta"), serde_json::to_vec(&meta)?)
    }

    pub fn set_message(&self, conversation: &str, m: &Stored) -> anyhow::Result<()> {
        check_id(conversation)?;
        if m.id.is_empty() || m.id.contains('/') {
            anyhow::bail!("a message id is not empty and has no `/`");
        }
        self.put(&format!("{conversation}/m/{}", m.id), serde_json::to_vec(m)?)
    }

    /// Deletes a conversation on every device: a tombstone `meta` with nothing else in it (not
    /// even the title), which Ymir reads as deleted too. Each Bragi device then drops the
    /// conversation's messages from its own log ([`ContextStore::compact`]).
    pub fn delete(&self, conversation: &str) -> anyhow::Result<()> {
        self.set_meta(conversation, Meta { deleted: true, ..Meta::default() })
    }

    /// Deletes one message on every device: its record's latest value becomes a tombstone
    /// (`{"deleted":true}`, no role or text), which neither Bragi nor Ymir reads as a message, so
    /// it is not shown or sent to a model again. [`ContextStore::compact`] drops the old values.
    pub fn delete_message(&self, conversation: &str, message: &str) -> anyhow::Result<()> {
        check_id(conversation)?;
        if message.is_empty() || message.contains('/') {
            anyhow::bail!("a message id is not empty and has no `/`");
        }
        let tomb = serde_json::json!({ "deleted": true, "at_ms": now_ms() });
        self.put(&format!("{conversation}/m/{message}"), serde_json::to_vec(&tomb)?)
    }

    /// Drops what no one can see any more from this device's copy: old values of every record,
    /// and every message of a deleted conversation (waiting writes included). Returns how many
    /// log entries were dropped. See [`Identity::compact`].
    pub fn compact(&self) -> anyhow::Result<usize> {
        let gone: std::collections::BTreeSet<String> = self.read_all().into_values().filter(|c| c.meta.deleted).map(|c| c.id).collect();
        let of_gone = |record: &str| record.split_once("/m/").is_some_and(|(c, _)| gone.contains(c));
        let before = self.pending().len();
        self.pending().retain(|k, _| !of_gone(k));
        if self.pending().len() != before {
            self.save_pending();
        }
        self.id.compact(CONTEXT_TYPE, &of_gone)
    }

    /// Live conversations, newest first by `updated_ms`.
    pub fn list(&self) -> Vec<StoredConversation> {
        let mut all: Vec<StoredConversation> =
            self.read_all().into_values().filter(|c| !c.meta.deleted && (!c.messages.is_empty() || !c.meta.title.is_empty())).collect();
        all.sort_by(|a, b| updated(b).cmp(&updated(a)).then_with(|| a.id.cmp(&b.id)));
        all
    }

    pub fn load(&self, conversation: &str) -> Option<StoredConversation> {
        self.load_any(conversation).filter(|c| !c.meta.deleted)
    }

    fn load_any(&self, conversation: &str) -> Option<StoredConversation> {
        self.read_all().remove(conversation)
    }

    fn read_all(&self) -> BTreeMap<String, StoredConversation> {
        let mut records: BTreeMap<String, Vec<u8>> = self.id.records(CONTEXT_TYPE).into_iter().collect();
        records.extend(self.pending().iter().map(|(k, v)| (k.clone(), v.clone())));
        let mut all: BTreeMap<String, StoredConversation> = BTreeMap::new();
        for (id, value) in records {
            let mut parts = id.splitn(3, '/');
            let (Some(conv), Some(kind)) = (parts.next(), parts.next()) else { continue };
            let c = all.entry(conv.to_string()).or_insert_with(|| StoredConversation {
                id: conv.to_string(),
                meta: Meta::default(),
                messages: Vec::new(),
            });
            match (kind, parts.next()) {
                ("meta", None) => {
                    if let Ok(m) = serde_json::from_slice(&value) {
                        c.meta = m;
                    }
                }
                ("m", Some(mid)) => {
                    if let Ok(mut m) = serde_json::from_slice::<Stored>(&value) {
                        m.id = mid.to_string();
                        c.messages.push(m);
                    }
                }
                _ => {}
            }
        }
        for c in all.values_mut() {
            c.messages.sort_by(|a, b| (a.at_ms, &a.id).cmp(&(b.at_ms, &b.id)));
        }
        all
    }

    /// A cheap fingerprint of what is stored, to tell whether a sync pass changed anything.
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (k, v) in self.id.records(CONTEXT_TYPE) {
            k.hash(&mut h);
            v.hash(&mut h);
        }
        h.finish()
    }
}

fn updated(c: &StoredConversation) -> i64 {
    c.meta.ext.get("updated_ms").and_then(Value::as_i64).unwrap_or_else(|| c.messages.last().map_or(c.meta.at_ms, |m| m.at_ms.max(c.meta.at_ms)))
}

fn check_id(conversation: &str) -> anyhow::Result<()> {
    if conversation.is_empty() || conversation.contains('/') {
        anyhow::bail!("a conversation id is not empty and has no `/`");
    }
    Ok(())
}

pub fn updated_ms(c: &StoredConversation) -> i64 {
    updated(c)
}
