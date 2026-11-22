//! The file picker: a tree of the code-mode root to add files to the context from. This is its
//! state (the tree, what is open, the filter and the cursor); `ui.rs` draws it and `app.rs`
//! feeds it keys and turns Space and Enter into `CodeState::set_file`.

use std::collections::HashSet;
use std::path::Path;

/// Folders that are never listed, whatever `.gitignore` says.
pub const HIDDEN_DIRS: &[&str] =
    &[".git", ".hg", ".svn", "target", "node_modules", "dist", "build", ".dart_tool", "__pycache__", ".venv", "venv", ".idea", ".gradle", "Pods"];

/// How many entries the tree holds; a bigger folder is cut off and says so.
pub const MAX_ENTRIES: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Relative to the root, `/`-separated.
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Browse,
    /// Typing a filter: letters go to it.
    Filter,
}

#[derive(Debug)]
pub struct Picker {
    nodes: Vec<Node>,
    pub truncated: bool,
    open: HashSet<String>,
    pub filter: String,
    pub mode: Mode,
    /// Index into `rows()`.
    pub sel: usize,
    pub scroll: usize,
}

/// Lists `root` the way git would see it: `.gitignore`, global ignores and dotfiles are
/// respected, `HIDDEN_DIRS` never appear, folders come first.
pub fn build(root: &Path) -> (Vec<Node>, bool) {
    let mut nodes = Vec::new();
    let mut truncated = false;
    let walk = ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|e| {
            !(e.file_type().is_some_and(|t| t.is_dir()) && e.depth() > 0 && HIDDEN_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
        })
        .sort_by_file_path(|a, b| {
            b.is_dir()
                .cmp(&a.is_dir())
                .then_with(|| a.file_name().map(|n| n.to_ascii_lowercase()).cmp(&b.file_name().map(|n| n.to_ascii_lowercase())))
        })
        .build();
    for entry in walk.flatten() {
        if entry.depth() == 0 {
            continue;
        }
        if nodes.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else { continue };
        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
        let is_file = entry.file_type().is_some_and(|t| t.is_file());
        if !is_dir && !is_file {
            continue;
        }
        let path = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
        nodes.push(Node {
            name: entry.file_name().to_string_lossy().into_owned(),
            depth: entry.depth() - 1,
            is_dir,
            size: if is_file { entry.metadata().map(|m| m.len()).unwrap_or(0) } else { 0 },
            path,
        });
    }
    // Folders with nothing under them aren't worth a row.
    let mut keep = vec![true; nodes.len()];
    for i in (0..nodes.len()).rev() {
        if nodes[i].is_dir && !nodes.get(i + 1).is_some_and(|n| n.depth > nodes[i].depth) {
            keep[i] = false;
        }
    }
    let mut it = keep.iter();
    nodes.retain(|_| *it.next().unwrap());
    (nodes, truncated)
}

/// A fuzzy match of `query` in `path`: `None` when its letters don't occur in order, else a
/// score (higher is better): runs of letters, starts of words and the file name count more.
pub fn fuzzy(query: &str, path: &str) -> Option<i64> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if q.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = path.chars().collect();
    let name_start = path.rfind('/').map_or(0, |i| path[..=i].chars().count());
    let (mut qi, mut score, mut prev) = (0usize, 0i64, None::<usize>);
    for (i, c) in chars.iter().enumerate() {
        if qi < q.len() && c.to_lowercase().next() == Some(q[qi]) {
            score += 1;
            if prev == Some(i.wrapping_sub(1)) {
                score += 4;
            }
            if i == 0 || matches!(chars[i - 1], '/' | '_' | '-' | '.' | ' ') {
                score += 3;
            }
            if i >= name_start {
                score += 2;
            }
            prev = Some(i);
            qi += 1;
        }
    }
    (qi == q.len()).then(|| score * 4 - (chars.len() as i64 / 8))
}

impl Picker {
    pub fn new(root: &Path) -> Self {
        let (nodes, truncated) = build(root);
        Self::from_nodes(nodes, truncated)
    }

    pub fn from_nodes(nodes: Vec<Node>, truncated: bool) -> Self {
        let mut p = Self { nodes, truncated, open: HashSet::new(), filter: String::new(), mode: Mode::Browse, sel: 0, scroll: 0 };
        // Top-level folders start open.
        for n in p.nodes.iter().filter(|n| n.is_dir && n.depth == 0) {
            p.open.insert(n.path.clone());
        }
        p
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn file_count(&self) -> usize {
        self.nodes.iter().filter(|n| !n.is_dir).count()
    }

    pub fn is_open(&self, path: &str) -> bool {
        self.open.contains(path)
    }

    /// What is on screen, in order: the tree with its folders open or shut, or, while a filter
    /// is set, the files that match it, best first.
    pub fn rows(&self) -> Vec<&Node> {
        if !self.filter.trim().is_empty() {
            let mut hits: Vec<(i64, &Node)> =
                self.nodes.iter().filter(|n| !n.is_dir).filter_map(|n| fuzzy(&self.filter, &n.path).map(|s| (s, n))).collect();
            hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.path.cmp(&b.1.path)));
            return hits.into_iter().map(|(_, n)| n).collect();
        }
        let mut rows = Vec::new();
        // The depth below which everything is folded away, while inside a shut folder.
        let mut hidden_below: Option<usize> = None;
        for n in &self.nodes {
            if let Some(d) = hidden_below {
                if n.depth > d {
                    continue;
                }
                hidden_below = None;
            }
            rows.push(n);
            if n.is_dir && !self.open.contains(&n.path) {
                hidden_below = Some(n.depth);
            }
        }
        rows
    }

    pub fn current(&self) -> Option<&Node> {
        self.rows().get(self.sel).copied()
    }

    pub fn clamp(&mut self) {
        let n = self.rows().len();
        self.sel = self.sel.min(n.saturating_sub(1));
    }

    pub fn step(&mut self, delta: isize) {
        let n = self.rows().len();
        if n == 0 {
            return;
        }
        self.sel = (self.sel as isize + delta).clamp(0, n as isize - 1) as usize;
    }

    pub fn first(&mut self) {
        self.sel = 0;
    }

    pub fn last(&mut self) {
        self.sel = self.rows().len().saturating_sub(1);
    }

    /// Opens or shuts the folder under the cursor.
    pub fn toggle_dir(&mut self) {
        let Some(n) = self.current().filter(|n| n.is_dir).cloned() else { return };
        if !self.open.remove(&n.path) {
            self.open.insert(n.path);
        }
    }

    /// Right: open a folder. Left: shut it, or move to the folder holding the cursor's row.
    pub fn expand(&mut self) {
        if let Some(n) = self.current().filter(|n| n.is_dir).cloned() {
            self.open.insert(n.path);
        }
    }

    pub fn collapse(&mut self) {
        let Some(n) = self.current().cloned() else { return };
        if n.is_dir && self.open.contains(&n.path) {
            self.open.remove(&n.path);
            return;
        }
        if let Some((parent, _)) = n.path.rsplit_once('/') {
            if let Some(i) = self.rows().iter().position(|r| r.is_dir && r.path == parent) {
                self.sel = i;
            }
        }
    }

    /// The files under a folder row (or just the file).
    pub fn files_under(&self, node: &Node) -> Vec<&Node> {
        if !node.is_dir {
            return self.nodes.iter().filter(|n| n.path == node.path).collect();
        }
        let prefix = format!("{}/", node.path);
        self.nodes.iter().filter(|n| !n.is_dir && n.path.starts_with(&prefix)).collect()
    }

    pub fn type_char(&mut self, c: char) {
        self.filter.push(c);
        self.sel = 0;
        self.scroll = 0;
    }

    pub fn backspace(&mut self) {
        self.filter.pop();
        self.sel = 0;
        self.scroll = 0;
    }

    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.mode = Mode::Browse;
        self.clamp();
    }

    /// Keeps the cursor inside a window of `height` rows; returns the first row to draw.
    pub fn window(&mut self, height: usize) -> usize {
        if height == 0 {
            return 0;
        }
        if self.sel < self.scroll {
            self.scroll = self.sel;
        } else if self.sel >= self.scroll + height {
            self.scroll = self.sel + 1 - height;
        }
        self.scroll
    }
}

/// `1.2 KiB`, `340 B`, `5 MiB`: a file size as the picker shows it.
pub fn size(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KiB", b as f64 / 1024.0),
        b => format!("{:.1} MiB", b as f64 / 1048576.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        for (p, body) in [
            ("src/main.rs", "fn main() {}"),
            ("src/lib.rs", "pub fn a() {}"),
            ("src/deep/mod.rs", "mod x;"),
            ("docs/guide.md", "# guide"),
            ("README.md", "hi"),
            ("target/debug/out", "bin"),
            ("node_modules/x/index.js", "x"),
            (".git/config", "[core]"),
            ("run.log", "log"),
            (".env", "SECRET=1"),
        ] {
            let f = r.join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, body).unwrap();
        }
        std::fs::write(r.join(".gitignore"), "*.log\n").unwrap();
        std::fs::create_dir_all(r.join("empty")).unwrap();
        d
    }

    fn paths(p: &Picker) -> Vec<String> {
        p.rows().iter().map(|n| n.path.clone()).collect()
    }

    #[test]
    fn the_tree_follows_gitignore_and_hides_build_folders() {
        let d = project();
        let (nodes, truncated) = build(d.path());
        assert!(!truncated);
        let all: Vec<&str> = nodes.iter().map(|n| n.path.as_str()).collect();
        // Folders first, then files, each by name.
        assert_eq!(all, ["docs", "docs/guide.md", "src", "src/deep", "src/deep/mod.rs", "src/lib.rs", "src/main.rs", "README.md"]);
        for hidden in ["target", "node_modules", ".git", "run.log", ".env", "empty"] {
            assert!(!all.iter().any(|p| p.starts_with(hidden)), "{hidden} is listed");
        }
        let main = nodes.iter().find(|n| n.path == "src/main.rs").unwrap();
        assert_eq!((main.size, main.depth, main.is_dir, main.name.as_str()), (12, 1, false, "main.rs"));
    }

    #[test]
    fn folders_open_and_shut() {
        let d = project();
        let mut p = Picker::new(d.path());
        // Top-level folders start open, deeper ones shut.
        assert_eq!(paths(&p), ["docs", "docs/guide.md", "src", "src/deep", "src/lib.rs", "src/main.rs", "README.md"]);
        p.sel = 0;
        p.toggle_dir();
        assert_eq!(paths(&p), ["docs", "src", "src/deep", "src/lib.rs", "src/main.rs", "README.md"]);
        p.sel = 2;
        assert_eq!(p.current().unwrap().path, "src/deep");
        p.expand();
        assert!(paths(&p).contains(&"src/deep/mod.rs".to_string()));
        p.step(1);
        p.collapse();
        assert_eq!(p.current().unwrap().path, "src/deep", "left goes to the folder");
        p.collapse();
        assert!(!paths(&p).contains(&"src/deep/mod.rs".to_string()));
        p.last();
        assert_eq!(p.current().unwrap().path, "README.md");
        p.step(5);
        assert_eq!(p.sel, p.rows().len() - 1);
        p.first();
        p.step(-3);
        assert_eq!(p.sel, 0);
    }

    #[test]
    fn the_filter_is_fuzzy_and_ranks_names_first() {
        let d = project();
        let mut p = Picker::new(d.path());
        for c in "mai".chars() {
            p.type_char(c);
        }
        assert_eq!(paths(&p).first().map(String::as_str), Some("src/main.rs"));
        p.clear_filter();
        for c in "guide".chars() {
            p.type_char(c);
        }
        assert_eq!(paths(&p), ["docs/guide.md"]);
        p.type_char('z');
        assert!(p.rows().is_empty());
        p.clamp();
        assert!(p.current().is_none());
        p.backspace();
        assert_eq!(paths(&p), ["docs/guide.md"]);
        p.clear_filter();
        assert!(p.rows().len() > 3);
        assert_eq!(fuzzy("", "a"), Some(0));
        assert!(fuzzy("ab", "a/b").is_some() && fuzzy("ba", "ab").is_none());
        assert!(fuzzy("lib", "src/lib.rs") > fuzzy("lib", "library/other/x.rs"));
    }

    #[test]
    fn files_under_a_folder_and_sizes() {
        let d = project();
        let p = Picker::new(d.path());
        let src = p.rows().into_iter().find(|n| n.path == "src").unwrap().clone();
        let under: Vec<&str> = p.files_under(&src).iter().map(|n| n.path.as_str()).collect();
        assert_eq!(under, ["src/deep/mod.rs", "src/lib.rs", "src/main.rs"]);
        assert_eq!((size(12), size(2048), size(3 * 1024 * 1024)), ("12 B".into(), "2.0 KiB".into(), "3.0 MiB".into()));
        assert_eq!(p.file_count(), 5);
    }

    #[test]
    fn the_window_follows_the_cursor() {
        let nodes = (0..50).map(|i| Node { path: format!("f{i:02}"), name: format!("f{i:02}"), depth: 0, is_dir: false, size: 1 }).collect();
        let mut p = Picker::from_nodes(nodes, false);
        assert_eq!(p.window(10), 0);
        p.sel = 12;
        assert_eq!(p.window(10), 3);
        p.sel = 1;
        assert_eq!(p.window(10), 1);
    }
}
