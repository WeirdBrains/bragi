//! The sidebar: folders (nested, foldable) holding sessions, the same `Core::folders` and
//! `Core::sessions` the desktop's folder tree draws. Sessions outside any folder come last.

use std::collections::{HashMap, HashSet};

use bragi_core::{Folder, SessionSummary};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Folder { id: String, name: String, depth: usize, open: bool, chats: usize },
    Chat { session: SessionSummary, depth: usize },
}

impl Row {
    pub fn depth(&self) -> usize {
        match self {
            Row::Folder { depth, .. } | Row::Chat { depth, .. } => *depth,
        }
    }

    pub fn chat_id(&self) -> Option<&str> {
        match self {
            Row::Chat { session, .. } => Some(&session.id),
            Row::Folder { .. } => None,
        }
    }

    pub fn folder_id(&self) -> Option<&str> {
        match self {
            Row::Folder { id, .. } => Some(id),
            Row::Chat { .. } => None,
        }
    }
}

/// Rows in drawing order. `closed` holds the folders folded shut; sessions keep `sessions`'
/// order (newest first) and folders sort by name.
pub fn rows(folders: &[Folder], sessions: &[SessionSummary], closed: &HashSet<String>) -> Vec<Row> {
    let known: HashSet<&str> = folders.iter().map(|f| f.id.as_str()).collect();
    let mut kids: HashMap<Option<&str>, Vec<&Folder>> = HashMap::new();
    for f in folders {
        let parent = f.parent.as_deref().filter(|p| known.contains(p));
        kids.entry(parent).or_default().push(f);
    }
    for v in kids.values_mut() {
        v.sort_by_key(|f| f.name.to_lowercase());
    }
    let mut chats: HashMap<Option<&str>, Vec<&SessionSummary>> = HashMap::new();
    for s in sessions {
        chats.entry(s.folder.as_deref().filter(|f| known.contains(f))).or_default().push(s);
    }
    fn count(id: &str, kids: &HashMap<Option<&str>, Vec<&Folder>>, chats: &HashMap<Option<&str>, Vec<&SessionSummary>>) -> usize {
        chats.get(&Some(id)).map_or(0, Vec::len) + kids.get(&Some(id)).map_or(0, |v| v.iter().map(|f| count(&f.id, kids, chats)).sum())
    }
    #[allow(clippy::too_many_arguments)]
    fn walk<'a>(
        parent: Option<&'a str>,
        depth: usize,
        kids: &HashMap<Option<&'a str>, Vec<&'a Folder>>,
        chats: &HashMap<Option<&'a str>, Vec<&'a SessionSummary>>,
        closed: &HashSet<String>,
        out: &mut Vec<Row>,
    ) {
        for f in kids.get(&parent).into_iter().flatten() {
            let open = !closed.contains(&f.id);
            out.push(Row::Folder { id: f.id.clone(), name: f.name.clone(), depth, open, chats: count(&f.id, kids, chats) });
            if open {
                walk(Some(&f.id), depth + 1, kids, chats, closed, out);
            }
        }
        for s in chats.get(&parent).into_iter().flatten() {
            out.push(Row::Chat { session: (*s).clone(), depth });
        }
    }
    let mut out = Vec::new();
    walk(None, 0, &kids, &chats, closed, &mut out);
    out
}

/// A folder named by the user: its id, or the one whose name matches (case-insensitive).
pub fn find_folder<'a>(folders: &'a [Folder], name: &str) -> Option<&'a Folder> {
    let name = name.trim();
    folders.iter().find(|f| f.id == name).or_else(|| folders.iter().find(|f| f.name.eq_ignore_ascii_case(name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(id: &str, folder: Option<&str>) -> SessionSummary {
        SessionSummary {
            id: id.into(),
            title: id.into(),
            updated_ms: 0,
            messages: 1,
            folder: folder.map(Into::into),
            target: None,
            target_label: "Weirdnet pool".into(),
            unbound: false,
        }
    }

    fn f(id: &str, name: &str, parent: Option<&str>) -> Folder {
        Folder { id: id.into(), name: name.into(), parent: parent.map(Into::into) }
    }

    #[test]
    fn nests_folders_and_folds_them() {
        let folders = vec![f("b", "Work", None), f("a", "Home", None), f("c", "Rust", Some("b"))];
        let sessions = vec![s("1", Some("c")), s("2", None), s("3", Some("a")), s("4", Some("gone"))];
        let r = rows(&folders, &sessions, &HashSet::new());
        let names: Vec<String> = r
            .iter()
            .map(|r| match r {
                Row::Folder { name, depth, chats, .. } => format!("{depth}{name}({chats})"),
                Row::Chat { session, depth } => format!("{depth}#{}", session.id),
            })
            .collect();
        assert_eq!(names, ["0Home(1)", "1#3", "0Work(1)", "1Rust(1)", "2#1", "0#2", "0#4"]);
        let closed: HashSet<String> = ["b".to_string()].into();
        assert_eq!(rows(&folders, &sessions, &closed).len(), 5);
        assert_eq!(find_folder(&folders, "rust").unwrap().id, "c");
    }
}
