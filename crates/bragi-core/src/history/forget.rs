//! Deleting history for real: messages (a question and the answer after it), whole
//! conversations, or all of it. A delete is a tombstone on the shared context store, so it
//! reaches the user's other devices with the next sync pass, and then this device's copy is
//! compacted ([`ContextStore::compact`]): the old values, their attachments and chunks leave the
//! op log on disk, and a deleted message is never shown or sent to a model again. Every Bragi
//! device compacts its own log after a sync pass brings a delete.
//!
//! The last delete can be undone for [`UNDO_MS`]: the removed messages are kept in memory only,
//! and undo writes them again (a later write wins over the tombstone on every device). Clearing
//! all history has no undo.
//!
//! Sessions and folders build on this: deleting a session is
//! [`History::delete_conversations`] over its conversations.

use super::*;

/// How long the last delete can be undone, in milliseconds.
pub const UNDO_MS: u64 = 10_000;

/// What a delete removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deleted {
    /// Pass to [`History::undo`] (`Core::undo_delete`) within [`UNDO_MS`].
    pub undo: String,
    pub conversations: usize,
    pub messages: usize,
}

pub(super) struct Undo {
    token: String,
    until_ms: i64,
    what: What,
}

enum What {
    Conversations(Vec<Conversation>),
    Messages { conversation: String, title: String, messages: Vec<ChatMessage> },
}

fn imported_dir(dir: &Path) -> PathBuf {
    dir.with_file_name("history.imported")
}

impl History {
    /// Deletes messages `ids` of `conversation`. A conversation whose first question goes gets
    /// its title from the next one (the title was that question's first line).
    pub fn delete_messages(&self, conversation: &str, ids: &[String]) -> Result<Deleted> {
        let c = self.load(conversation)?;
        if ids.is_empty() {
            return Err(CoreError::new(ErrorKind::BadRequest, "no message to delete"));
        }
        let mut gone: Vec<ChatMessage> = Vec::new();
        for id in ids {
            let m = c.messages.iter().find(|m| &m.id == id).ok_or_else(|| CoreError::new(ErrorKind::NotFound, format!("no message `{id}`")))?;
            if !gone.iter().any(|g| g.id == m.id) {
                gone.push(m.clone());
            }
        }
        let store = self.store()?;
        for m in &gone {
            store.delete_message(conversation, &m.id).map_err(storage)?;
        }
        let first = c.messages.iter().find(|m| m.role == Role::User);
        if let Some(first) = first.filter(|f| gone.iter().any(|g| g.id == f.id) && c.title == title_for(&f.text)) {
            let mut after = c.clone();
            after.messages.retain(|m| !gone.iter().any(|g| g.id == m.id));
            after.title =
                after.messages.iter().find(|m| m.role == Role::User && m.id != first.id).map_or_else(|| "New chat".into(), |m| title_for(&m.text));
            self.save(&after)?;
        }
        self.forget_imported(&[conversation]);
        self.compact();
        let n = gone.len();
        let undo = self.keep_for_undo(What::Messages { conversation: conversation.into(), title: c.title, messages: gone });
        Ok(Deleted { undo, conversations: 0, messages: n })
    }

    /// Deletes conversations `ids`; nothing is deleted when one of them isn't there.
    pub fn delete_conversations(&self, ids: &[String]) -> Result<Deleted> {
        let store = self.store()?;
        let mut gone = Vec::new();
        for id in ids {
            if !valid_id(id) {
                return Err(not_found(id));
            }
            let c = store.load(id).ok_or_else(|| not_found(id))?;
            if !gone.iter().any(|g: &Conversation| g.id == c.id) {
                gone.push(from_conversation(&c));
            }
        }
        for c in &gone {
            store.delete(&c.id).map_err(storage)?;
        }
        let ids: Vec<&str> = gone.iter().map(|c| c.id.as_str()).collect();
        self.forget_imported(&ids);
        self.compact();
        let (conversations, messages) = (gone.len(), gone.iter().map(|c| c.messages.len()).sum());
        let undo = self.keep_for_undo(What::Conversations(gone));
        Ok(Deleted { undo, conversations, messages })
    }

    /// Deletes every conversation, with no undo, and the files B1 left behind. Returns how many.
    pub fn clear(&self) -> Result<usize> {
        let store = self.store()?;
        let all = store.list();
        for c in &all {
            store.delete(&c.id).map_err(storage)?;
        }
        *self.undo.lock().unwrap_or_else(|e| e.into_inner()) = None;
        for d in [imported_dir(&self.history_dir()), self.history_dir()] {
            if d.exists()
                && let Err(e) = std::fs::remove_dir_all(&d)
            {
                tracing::warn!("removing {}: {e}", d.display());
            }
        }
        self.compact();
        Ok(all.len())
    }

    /// Puts back what the delete `token` names, if it is still the last one and in time. Returns
    /// how many messages came back.
    pub fn undo(&self, token: &str) -> Result<usize> {
        let u = self
            .undo
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take_if(|u| u.token == token && now_ms() <= u.until_ms)
            .ok_or_else(|| CoreError::new(ErrorKind::NotFound, "too late to undo that delete"))?;
        match u.what {
            What::Conversations(all) => {
                let mut n = 0;
                for c in &all {
                    self.save(c)?;
                    n += c.messages.len();
                }
                Ok(n)
            }
            What::Messages { conversation, title, messages } => {
                let mut c = self.load(&conversation)?;
                c.title = title;
                let n = messages.len();
                for m in messages {
                    if !c.messages.iter().any(|x| x.id == m.id) {
                        c.messages.push(m);
                    }
                }
                c.messages.sort_by(|a, b| (a.at_ms, &a.id).cmp(&(b.at_ms, &b.id)));
                self.save(&c)?;
                Ok(n)
            }
        }
    }

    /// Drops the undo copy of delete `token` (its time is up), so nothing of it stays in memory.
    pub fn forget_undo(&self, token: &str) {
        let mut u = self.undo.lock().unwrap_or_else(|e| e.into_inner());
        if u.as_ref().is_some_and(|u| u.token == token) {
            *u = None;
        }
    }

    /// Compacts this device's copy of the store (see [`ContextStore::compact`]). Returns how many
    /// log entries were dropped.
    pub fn compact(&self) -> usize {
        let Ok(store) = self.store() else { return 0 };
        match store.compact() {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("compacting history: {e:#}");
                0
            }
        }
    }

    fn keep_for_undo(&self, what: What) -> String {
        let token = crate::core::random_id("u");
        let until_ms = now_ms() + UNDO_MS as i64;
        *self.undo.lock().unwrap_or_else(|e| e.into_inner()) = Some(Undo { token: token.clone(), until_ms, what });
        token
    }

    /// Removes B1's imported copies of `ids` (they still hold every message).
    fn forget_imported(&self, ids: &[&str]) {
        let Ok(rd) = std::fs::read_dir(imported_dir(&self.history_dir())) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string();
            let id = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<Conversation>(&b).ok()).map_or(stem, |c| c.id);
            if ids.contains(&id.as_str())
                && let Err(e) = std::fs::remove_file(&p)
            {
                tracing::warn!("removing {}: {e}", p.display());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{conv, linked};
    use super::*;
    use crate::identity::CONTEXT_TYPE;

    /// Every value of the context type this device's log can still open, and how many chunk
    /// entries it holds.
    fn on_disk(id: &Identity) -> (Vec<String>, usize) {
        let st = id.state();
        let st = st.lock().unwrap();
        let ctyp = weirdnet_sync::chunk_type(CONTEXT_TYPE);
        let values = st
            .log
            .iter()
            .filter(|e| e.typ == CONTEXT_TYPE)
            .filter_map(|e| match st.read(e) {
                Ok(weirdnet_sync::RecordValue::Ready(v)) => Some(String::from_utf8_lossy(&v).into_owned()),
                _ => None,
            })
            .collect();
        (values, st.log.iter().filter(|e| e.typ == ctyp).count())
    }

    fn texts(c: &Conversation) -> Vec<&str> {
        c.messages.iter().map(|m| m.text.as_str()).collect()
    }

    #[test]
    fn a_question_and_its_answer_go_and_are_not_sent_again() {
        let tmp = tempfile::tempdir().unwrap();
        let (id, h) = linked(tmp.path());
        h.save(&conv("c", &["first question", "first answer", "secret question", "secret answer", "last", "last answer"])).unwrap();
        let c = h.load("c").unwrap();
        let q = &c.messages[2];
        let reply = c.reply_to(&q.id).unwrap().id.clone();
        assert_eq!(c.reply_to(&c.messages[1].id), None, "an answer has no reply");
        let d = h.delete_messages("c", &[q.id.clone(), reply]).unwrap();
        assert_eq!((d.conversations, d.messages), (0, 2));
        let c = h.load("c").unwrap();
        assert_eq!(texts(&c), ["first question", "first answer", "last", "last answer"]);
        // What the model gets on the next turn.
        let sent = crate::core::agent_messages(&c.messages);
        assert_eq!(sent.len(), 4);
        assert!(!format!("{sent:?}").contains("secret"));
        // Old values are gone from the log on disk, after a restart too.
        let (values, _) = on_disk(&id);
        assert!(values.iter().all(|v| !v.contains("secret")), "{values:?}");
        drop((id, h));
        let id = Arc::new(Identity::open(&tmp.path().join("identity")).unwrap().unwrap());
        assert!(on_disk(&id).0.iter().all(|v| !v.contains("secret")));
        let h = History::open(tmp.path()).unwrap();
        h.attach(id.clone());
        assert_eq!(texts(&h.load("c").unwrap()), ["first question", "first answer", "last", "last answer"]);
        // Writes carry on after the compacted log.
        let mut c = h.load("c").unwrap();
        c.messages.push(ChatMessage { id: String::new(), text: "more".into(), at_ms: 3000, ..c.messages[0].clone() });
        h.save(&c).unwrap();
        assert_eq!(h.load("c").unwrap().messages.len(), 5);
    }

    #[test]
    fn deleting_the_first_question_retitles_and_undo_puts_it_back() {
        let tmp = tempfile::tempdir().unwrap();
        let (_id, h) = linked(tmp.path());
        h.save(&conv("c", &["private first line", "answer", "second question", "answer 2"])).unwrap();
        let c = h.load("c").unwrap();
        let d = h.delete_messages("c", &[c.messages[0].id.clone()]).unwrap();
        let after = h.load("c").unwrap();
        assert_eq!(after.title, "second question");
        assert_eq!(h.page(0, 5).unwrap()[0].title, "second question");
        assert_eq!(h.undo(&d.undo).unwrap(), 1);
        let back = h.load("c").unwrap();
        assert_eq!((back.title.as_str(), texts(&back)), ("private first line", vec!["private first line", "answer", "second question", "answer 2"]));
        assert_eq!(h.undo(&d.undo).unwrap_err().kind, ErrorKind::NotFound, "one undo");
    }

    #[test]
    fn a_conversation_goes_with_its_attachments_and_chunks_and_can_come_back() {
        let tmp = tempfile::tempdir().unwrap();
        let (id, h) = linked(tmp.path());
        let mut c = conv("big", &["read this", "done"]);
        // Big enough to be chunked.
        let text: String = (0..6000).map(|i| format!("attached line {i}\n")).collect();
        c.messages[0].attachments.push(crate::attach::Attachment { name: "notes.md".into(), text, media: None });
        h.save(&c).unwrap();
        h.save(&conv("keep", &["kept", "yes"])).unwrap();
        assert!(on_disk(&id).1 > 0, "the attachment is chunked");
        let d = h.delete_conversations(&["big".into()]).unwrap();
        assert_eq!((d.conversations, d.messages), (1, 2));
        assert_eq!(h.load("big").unwrap_err().kind, ErrorKind::NotFound);
        let (values, chunks) = on_disk(&id);
        assert_eq!(chunks, 0);
        assert!(values.iter().all(|v| !v.contains("read this") && !v.contains("attached line")), "{values:?}");
        assert_eq!(h.page(0, 5).unwrap().len(), 1);
        // Undo writes it again, attachment and all.
        h.undo(&d.undo).unwrap();
        let back = h.load("big").unwrap();
        assert_eq!((back.title.as_str(), back.messages.len(), back.messages[0].attachments.len()), ("read this", 2, 1));
        assert_eq!(h.delete_conversations(&["big".into(), "nope".into()]).unwrap_err().kind, ErrorKind::NotFound);
        assert!(h.load("big").is_ok(), "nothing deleted when one is missing");
    }

    #[test]
    fn clear_takes_everything_and_the_b1_files() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("history");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("old-1.json"), serde_json::to_vec(&conv("old-1", &["from before", "yes"])).unwrap()).unwrap();
        let (id, h) = linked(tmp.path());
        assert!(tmp.path().join("history.imported").is_dir());
        h.save(&conv("a", &["alpha secret", "x"])).unwrap();
        let d = h.delete_conversations(&["a".into()]).unwrap();
        h.undo(&d.undo).unwrap();
        let d = h.delete_conversations(&["a".into()]).unwrap();
        assert_eq!(h.clear().unwrap(), 1);
        assert!(h.page(0, 10).unwrap().is_empty());
        assert!(!tmp.path().join("history.imported").exists());
        assert_eq!(h.undo(&d.undo).unwrap_err().kind, ErrorKind::NotFound, "no undo after clearing");
        let (values, _) = on_disk(&id);
        assert!(values.iter().all(|v| !v.contains("secret") && !v.contains("from before")), "{values:?}");
        // Only the tombstones are left, with no titles.
        assert!(values.iter().all(|v| v.contains("\"deleted\":true")), "{values:?}");
    }

    #[test]
    fn undo_runs_out() {
        let tmp = tempfile::tempdir().unwrap();
        let (_id, h) = linked(tmp.path());
        h.save(&conv("c", &["q", "a"])).unwrap();
        let d = h.delete_conversations(&["c".into()]).unwrap();
        h.forget_undo(&d.undo);
        assert_eq!(h.undo(&d.undo).unwrap_err().kind, ErrorKind::NotFound);
        assert_eq!(h.undo("u-unknown").unwrap_err().kind, ErrorKind::NotFound);
    }
}
