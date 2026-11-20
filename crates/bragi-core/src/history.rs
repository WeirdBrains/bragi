//! Local history (B1): one JSON file per conversation under `<dir>/history/`. B4 moves history
//! onto the synced `weirdnet-agent` context store; the shapes here are what the shells draw.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, ErrorKind, Result};

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

pub struct History {
    dir: PathBuf,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

impl History {
    pub fn open(dir: &Path) -> Result<Self> {
        let dir = dir.join("history");
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn path(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            return Err(CoreError::new(ErrorKind::NotFound, format!("no conversation `{id}`")));
        }
        Ok(self.dir.join(format!("{id}.json")))
    }

    pub fn load(&self, id: &str) -> Result<Conversation> {
        let bytes = std::fs::read(self.path(id)?).map_err(|_| CoreError::new(ErrorKind::NotFound, format!("no conversation `{id}`")))?;
        serde_json::from_slice(&bytes).map_err(|e| CoreError::new(ErrorKind::Storage, format!("conversation {id}: {e}")))
    }

    pub fn save(&self, c: &Conversation) -> Result<()> {
        let path = self.path(&c.id)?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(c).map_err(|e| CoreError::new(ErrorKind::Storage, e.to_string()))?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        std::fs::remove_file(self.path(id)?)?;
        Ok(())
    }

    /// Newest first: `offset` and `limit` page through them.
    pub fn page(&self, offset: usize, limit: usize) -> Result<Vec<ConversationSummary>> {
        let mut all = Vec::new();
        for e in std::fs::read_dir(&self.dir)? {
            let p = e?.path();
            if p.extension().is_some_and(|x| x == "json")
                && let Ok(b) = std::fs::read(&p)
                && let Ok(c) = serde_json::from_slice::<Conversation>(&b)
            {
                all.push(c.summary());
            }
        }
        all.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms).then_with(|| a.id.cmp(&b.id)));
        Ok(all.into_iter().skip(offset).take(limit).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_page_load_and_refuse_odd_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let h = History::open(tmp.path()).unwrap();
        for (i, id) in ["a1", "b2", "c3"].iter().enumerate() {
            let c = Conversation { id: id.to_string(), title: title_for("hi"), created_ms: 0, updated_ms: i as i64, messages: vec![] };
            h.save(&c).unwrap();
        }
        let page = h.page(0, 2).unwrap();
        assert_eq!(page.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["c3", "b2"]);
        assert_eq!(h.page(2, 5).unwrap().len(), 1);
        assert_eq!(h.load("b2").unwrap().updated_ms, 1);
        assert_eq!(h.load("../x").unwrap_err().kind, ErrorKind::NotFound);
        h.delete("a1").unwrap();
        assert_eq!(h.page(0, 10).unwrap().len(), 2);
        assert_eq!(title_for(&"x".repeat(60)).chars().count(), 49);
    }
}
