//! What Bragi asks the model for code, and how it reads the answer.
//!
//! Every proposed change comes back as a fenced block tagged `bragi-edit`:
//!
//! ````text
//! ```bragi-edit path=src/lib.rs lines=10-14
//! the new text for lines 10 to 14
//! ```
//! ````
//!
//! `lines=A-B` replaces lines A..=B (1-based), `after=N` inserts after line N (`after=0` is the
//! top), and no `lines`/`after` replaces (or creates) the whole file. Text outside the blocks is
//! the model's explanation, shown as is. For a rewrite, an answer with no `bragi-edit` block but
//! one plain fenced block is read as the new text for the target.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use super::context::{ContextFile, looks_secret};
use super::diff::{FileDiff, lines};
use super::editor::{PromptMessage, fence_for, strip_thinking};
use crate::error::{CoreError, ErrorKind, Result};

/// The fence tag of a proposed change.
pub const EDIT_TAG: &str = "bragi-edit";

/// Lines `start..=end`, 1-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRange {
    pub start: usize,
    pub end: usize,
}

impl LineRange {
    pub fn new(start: usize, end: usize) -> Result<Self> {
        if start == 0 || end < start {
            return Err(bad(format!("bad line range {start}-{end}: lines start at 1 and the end can't come before the start")));
        }
        Ok(Self { start, end })
    }

    /// `10-40`, `10` or `10:40`.
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        let (a, b) = s.split_once(['-', ':', ',']).unwrap_or((s, s));
        let n = |x: &str| x.trim().parse::<usize>().map_err(|_| bad(format!("bad line range {s:?}")));
        Self::new(n(a)?, n(b)?)
    }
}

impl std::fmt::Display for LineRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.start == self.end { write!(f, "{}", self.start) } else { write!(f, "{}-{}", self.start, self.end) }
    }
}

/// A file, or lines of one, to rewrite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub path: String,
    pub range: Option<LineRange>,
}

/// `src/lib.rs`, `src/lib.rs:10-40` or `src/lib.rs:12`. A Windows drive (`C:\x.rs`) is not a range.
pub fn parse_target(s: &str) -> Result<Target> {
    let s = s.trim();
    if let Some((path, range)) = s.rsplit_once(':')
        && !path.is_empty()
        && !range.is_empty()
        && range.chars().all(|c| c.is_ascii_digit() || c == '-')
    {
        return Ok(Target { path: path.to_string(), range: Some(LineRange::parse(range)?) });
    }
    if s.is_empty() {
        return Err(bad("name a file to rewrite"));
    }
    Ok(Target { path: s.to_string(), range: None })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "task", rename_all = "snake_case")]
pub enum CodeTask {
    /// Review the context and propose changes, each as an edit block.
    Suggest,
    /// Rewrite one file or range following the instruction.
    Rewrite(Target),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeRequest {
    pub task: CodeTask,
    pub instruction: String,
    pub files: Vec<ContextFile>,
}

/// Where a proposed change goes in its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "span", rename_all = "snake_case")]
pub enum Span {
    Lines(LineRange),
    /// Insert after this line (0 = the top).
    After {
        line: usize,
    },
    Whole,
}

/// One change the model proposed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedEdit {
    pub path: String,
    pub span: Span,
    /// The new lines, each ending in `\n`.
    pub text: String,
}

fn bad(msg: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::BadRequest, msg)
}

/// How one context file appears in the prompt: line-numbered, in its own fence. [`build`] joins
/// these, so a file's share of the prompt is exactly this block (see `budget`).
pub(crate) fn file_block(f: &ContextFile) -> String {
    let body = numbered(&f.text);
    let fence = fence_for(&body);
    format!("\n{fence}text path={}\n{body}{fence}\n", f.path)
}

fn numbered(text: &str) -> String {
    let width = lines(text).len().max(1).to_string().len();
    let mut s = String::with_capacity(text.len() + text.len() / 4);
    for (i, l) in text.lines().enumerate() {
        s.push_str(&format!("{:>width$}| {l}\n", i + 1));
    }
    s
}

const FORMAT: &str = "Give every change as a fenced block whose info string is `bragi-edit` followed by `path=` and where it goes:\n\
- `lines=A-B` replaces lines A to B (1-based, inclusive) with the block's text,\n\
- `after=N` inserts the block's text after line N (`after=0` is the top of the file),\n\
- neither replaces the whole file (or creates it).\n\
For example:\n\
```bragi-edit path=src/app.rs lines=12-14\n\
fn ready(&self) -> bool {\n    self.state == State::Ready\n}\n\
```\n\
The line numbers shown with the files are for reference only: never put them in a block. Keep the file's indentation and style. Before each block, say in a sentence or two what it changes and why.";

const SUGGEST_SYSTEM: &str =
    "You review code and propose concrete, focused changes (fixes, clarity, simplifications); skip changes that are only taste.";

const REWRITE_SYSTEM: &str = "You rewrite code as asked. Change only what the instruction asks for, keep everything else, keep the style and indentation, and never leave placeholders such as \"rest unchanged\".";

/// A one-line summary of `req` for a shell to show (`Rewrite lines 10-40 of src/lib.rs: …`).
pub fn title(req: &CodeRequest) -> String {
    let i = req.instruction.trim();
    match &req.task {
        CodeTask::Suggest if i.is_empty() => "Code suggestions".into(),
        CodeTask::Suggest => format!("Code suggestions: {i}"),
        CodeTask::Rewrite(Target { path, range: Some(r) }) => format!("Rewrite lines {r} of {path}: {i}"),
        CodeTask::Rewrite(Target { path, range: None }) => format!("Rewrite {path}: {i}"),
    }
}

/// The messages for `req`, for [`Core::complete`](crate::Core::complete).
pub fn build(req: &CodeRequest) -> Result<Vec<PromptMessage>> {
    let instruction = req.instruction.trim();
    let mut files = if req.files.is_empty() {
        String::from("The project has no files yet: create the ones the request needs, each as a whole-file block (no `lines` or `after`).\n")
    } else {
        String::from("The files (line-numbered for reference):\n")
    };
    for f in &req.files {
        files.push_str(&file_block(f));
    }
    let (system, ask) = match &req.task {
        CodeTask::Suggest => {
            // With no files, the request has to say what to make.
            if req.files.is_empty() && instruction.is_empty() {
                return Err(bad("add a file or folder to review, or say what to create"));
            }
            let what = if instruction.is_empty() { "Suggest improvements to this code." } else { instruction };
            (format!("{SUGGEST_SYSTEM}\n\n{FORMAT}"), format!("{what}\n"))
        }
        CodeTask::Rewrite(t) => {
            if instruction.is_empty() {
                return Err(bad("say how to rewrite it"));
            }
            let file = req.files.iter().find(|f| f.path == t.path).ok_or_else(|| bad(format!("{} is not among the files sent", t.path)))?;
            let count = file.text.lines().count();
            let (where_, attr) = match t.range {
                Some(r) => {
                    if r.end > count.max(1) {
                        return Err(bad(format!("{} has {count} lines; {r} is past the end", t.path)));
                    }
                    (format!("lines {r} of {}", t.path), format!("path={} lines={}-{}", t.path, r.start, r.end))
                }
                None => (t.path.clone(), format!("path={}", t.path)),
            };
            let system = format!(
                "{REWRITE_SYSTEM} Answer with the rewritten text in exactly one fenced block whose info string is `{EDIT_TAG}` followed by where it goes (here `{attr}`); the block replaces that text as a whole. The line numbers shown with the files are for reference only: never put them in the block. After the block, say in a sentence what changed."
            );
            let mut ask = format!("Rewrite {where_}: {instruction}\n");
            if let Some(r) = t.range {
                let selected: String = file.text.lines().skip(r.start - 1).take(r.end - r.start + 1).map(|l| format!("{l}\n")).collect();
                let f = fence_for(&selected);
                ask.push_str(&format!("\nThe lines to rewrite ({where_}):\n{f}\n{selected}{f}\n"));
            }
            ask.push_str(&format!("\nAnswer with one `{EDIT_TAG} {attr}` block."));
            (system, ask)
        }
    };
    Ok(vec![PromptMessage::system(system), PromptMessage::user(format!("{files}\n{ask}"))])
}

fn attrs(info: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut rest = info.trim();
    while !rest.is_empty() {
        let (tok, next) = match rest.find('=') {
            Some(eq) if !rest[..eq].contains(char::is_whitespace) => {
                let key = &rest[..eq];
                let after = &rest[eq + 1..];
                let (val, next) = if let Some(q) = after.strip_prefix('"') {
                    match q.find('"') {
                        Some(end) => (&q[..end], &q[end + 1..]),
                        None => (q, ""),
                    }
                } else {
                    let end = after.find(char::is_whitespace).unwrap_or(after.len());
                    (&after[..end], &after[end..])
                };
                ((key.to_string(), val.to_string()), next)
            }
            _ => {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                ((rest[..end].to_string(), String::new()), &rest[end..])
            }
        };
        out.insert(tok.0, tok.1);
        rest = next.trim_start();
    }
    out
}

/// Drops `  12| ` prefixes when every non-empty line has one (a model copying the numbered view).
pub(crate) fn strip_numbers(text: &str) -> String {
    let numbered = |l: &str| {
        let t = l.trim_start();
        let digits = t.chars().take_while(char::is_ascii_digit).count();
        digits > 0 && t[digits..].starts_with('|')
    };
    let body: Vec<&str> = text.lines().collect();
    if body.is_empty() || !body.iter().filter(|l| !l.trim().is_empty()).all(|l| numbered(l)) {
        return text.to_string();
    }
    body.iter()
        .map(|l| {
            let t = l.trim_start();
            let rest = &t[t.chars().take_while(char::is_ascii_digit).count() + 1..];
            format!("{}\n", rest.strip_prefix(' ').unwrap_or(rest))
        })
        .collect()
}

/// Fenced blocks in `reply`: `(info string, body)`, body lines ending in `\n`.
pub fn fenced_blocks(reply: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut open: Option<(char, usize, String, String)> = None;
    for line in reply.lines() {
        let t = line.trim_start();
        let ch = t.chars().next().unwrap_or(' ');
        let run = if ch == '`' || ch == '~' { t.chars().take_while(|c| *c == ch).count() } else { 0 };
        match &mut open {
            None if run >= 3 => open = Some((ch, run, t[run..].trim().to_string(), String::new())),
            None => {}
            Some((c, n, _, _)) if ch == *c && run >= *n && t[run..].trim().is_empty() => {
                let (_, _, info, body) = open.take().unwrap_or_default();
                out.push((info, body));
            }
            Some((_, _, _, body)) => {
                body.push_str(line);
                body.push('\n');
            }
        }
    }
    out
}

/// The `bragi-edit` blocks in `reply`. For a rewrite, a block without a path is for the target,
/// and an answer with no edit blocks but exactly one plain fenced block is read as the target's
/// new text.
pub fn parse_edits(reply: &str, task: &CodeTask) -> Result<Vec<ProposedEdit>> {
    let blocks = fenced_blocks(strip_thinking(reply));
    let target_span = |t: &Target| t.range.map(Span::Lines).unwrap_or(Span::Whole);
    let mut edits = Vec::new();
    for (info, body) in &blocks {
        let mut words = info.split_whitespace();
        if words.next() != Some(EDIT_TAG) {
            continue;
        }
        let a = attrs(&info[EDIT_TAG.len()..]);
        let text = strip_numbers(body);
        let (path, default_span) = match (a.get("path").filter(|p| !p.is_empty()), task) {
            (Some(p), _) => (p.clone(), Span::Whole),
            (None, CodeTask::Rewrite(t)) => (t.path.clone(), target_span(t)),
            (None, CodeTask::Suggest) => return Err(bad("the answer has an edit without a path")),
        };
        let span = if let Some(l) = a.get("lines") {
            Span::Lines(LineRange::parse(l)?)
        } else if let Some(n) = a.get("after") {
            Span::After { line: n.trim().parse().map_err(|_| bad(format!("bad after={n}")))? }
        } else {
            default_span
        };
        edits.push(ProposedEdit { path, span, text });
    }
    if edits.is_empty()
        && let CodeTask::Rewrite(t) = task
    {
        let plain: Vec<&(String, String)> = blocks.iter().collect();
        if let [(_, body)] = plain.as_slice() {
            edits.push(ProposedEdit { path: t.path.clone(), span: target_span(t), text: strip_numbers(body) });
        }
    }
    Ok(edits)
}

/// The answer without its edit blocks: the model's explanation.
pub fn explanation(reply: &str) -> String {
    let reply = strip_thinking(reply);
    let mut out = String::new();
    let mut skipping: Option<(char, usize)> = None;
    for line in reply.lines() {
        let t = line.trim_start();
        let ch = t.chars().next().unwrap_or(' ');
        let run = if ch == '`' || ch == '~' { t.chars().take_while(|c| *c == ch).count() } else { 0 };
        match skipping {
            None if run >= 3 && t[run..].trim_start().starts_with(EDIT_TAG) => skipping = Some((ch, run)),
            None => {
                out.push_str(line);
                out.push('\n');
            }
            Some((c, n)) if ch == c && run >= n && t[run..].trim().is_empty() => skipping = None,
            Some(_) => {}
        }
    }
    out.trim().to_string()
}

/// A path from the model that stays inside the project: relative, no `..`, not a secret.
pub fn safe_path(path: &str) -> Result<()> {
    let p = Path::new(path);
    if path.is_empty() || p.is_absolute() || path.starts_with('/') || path.starts_with('\\') || path.contains(':') {
        return Err(bad(format!("{path:?} is not a path inside the project")));
    }
    if p.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Err(bad(format!("{path:?} leaves the project")));
    }
    if looks_secret(p) {
        return Err(bad(format!("{path} looks like a secret; not editing it")));
    }
    Ok(())
}

/// `original` with `edits` (all for one file) applied. Edits refer to the original's line
/// numbers; overlapping edits are refused.
pub fn apply_edits(original: &str, edits: &[&ProposedEdit]) -> Result<String> {
    let old = lines(original);
    let n = old.len();
    // (start index, end index exclusive, text, order)
    let mut spans: Vec<(usize, usize, &str)> = Vec::new();
    for e in edits {
        let (s, t) = match e.span {
            Span::Whole => (0, n),
            Span::Lines(r) => {
                if r.start > n + 1 || (r.start <= n && r.end > n) {
                    return Err(bad(format!("{}: lines {r} are past the end ({n} lines)", e.path)));
                }
                (r.start - 1, r.end.min(n))
            }
            Span::After { line } => {
                if line > n {
                    return Err(bad(format!("{}: after={line} is past the end ({n} lines)", e.path)));
                }
                (line, line)
            }
        };
        spans.push((s, t, e.text.as_str()));
    }
    spans.sort_by_key(|(s, t, _)| (*s, *t));
    for w in spans.windows(2) {
        if w[1].0 < w[0].1 || (w[0].0 == w[0].1 && w[1].0 == w[0].0 && w[1].1 == w[1].0) {
            return Err(bad(format!("{}: the proposed changes overlap", edits[0].path)));
        }
    }
    let mut out = String::with_capacity(original.len());
    let mut at = 0;
    for (s, t, text) in &spans {
        for l in &old[at..*s] {
            out.push_str(l);
        }
        let mut text = text.to_string();
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        // Replacing the end of a file that had no final newline: keep it that way.
        if *t == n && n > 0 && !original.ends_with('\n') && *s < *t {
            text.pop();
        }
        out.push_str(&text);
        at = *t;
    }
    for l in &old[at..] {
        out.push_str(l);
    }
    Ok(out)
}

/// One diff per file from `edits`. The original text comes from `known` (the context sent) or
/// is read under `root`; a file that doesn't exist may only be created by a whole-file edit.
pub fn proposals(root: &Path, known: &[ContextFile], edits: &[ProposedEdit]) -> Result<Vec<FileDiff>> {
    let mut by_path: BTreeMap<&str, Vec<&ProposedEdit>> = BTreeMap::new();
    let mut order: Vec<&str> = Vec::new();
    for e in edits {
        safe_path(&e.path)?;
        if !by_path.contains_key(e.path.as_str()) {
            order.push(&e.path);
        }
        by_path.entry(&e.path).or_default().push(e);
    }
    let mut out = Vec::new();
    for path in order {
        let edits = &by_path[path];
        let (original, created) = match known.iter().find(|f| f.path == path) {
            Some(f) => (f.text.clone(), false),
            None => match std::fs::read_to_string(root.join(path)) {
                Ok(t) => (t, false),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if edits.iter().any(|e| e.span != Span::Whole) {
                        return Err(bad(format!("{path} doesn't exist, and the change edits lines of it")));
                    }
                    (String::new(), true)
                }
                Err(e) => return Err(CoreError::new(ErrorKind::Storage, format!("{path}: {e}"))),
            },
        };
        let new = apply_edits(&original, edits)?;
        let mut d = FileDiff::compute(path, original, new);
        d.created = created;
        if !d.is_empty() {
            out.push(d);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, text: &str) -> ContextFile {
        ContextFile { path: path.into(), text: text.into() }
    }

    #[test]
    fn targets_parse() {
        assert_eq!(parse_target("src/a.rs:10-40").unwrap(), Target { path: "src/a.rs".into(), range: Some(LineRange { start: 10, end: 40 }) });
        assert_eq!(parse_target("a.rs:7").unwrap().range, Some(LineRange { start: 7, end: 7 }));
        assert_eq!(parse_target("a.rs").unwrap().range, None);
        assert_eq!(parse_target(r"C:\x\a.rs").unwrap().path, r"C:\x\a.rs");
        assert!(parse_target("a.rs:9-3").is_err());
    }

    #[test]
    fn rewrite_prompt_names_the_target_and_numbers_lines() {
        let req = CodeRequest {
            task: CodeTask::Rewrite(parse_target("a.rs:2-3").unwrap()),
            instruction: "make it shorter".into(),
            files: vec![file("a.rs", "one\ntwo\nthree\nfour\n")],
        };
        assert_eq!(title(&req), "Rewrite lines 2-3 of a.rs: make it shorter");
        let p = build(&req).unwrap();
        assert_eq!(p.len(), 2);
        assert!(p[0].content.contains("`bragi-edit`"));
        assert!(p[1].content.contains("Rewrite lines 2-3 of a.rs: make it shorter\n"));
        assert!(p[1].content.contains("`bragi-edit path=a.rs lines=2-3`"));
        assert!(p[1].content.contains("2| two\n"));
        assert!(build(&CodeRequest { task: CodeTask::Rewrite(parse_target("a.rs:9").unwrap()), ..req }).is_err());
    }

    #[test]
    fn reads_edit_blocks_and_explanation() {
        let reply =
            "Tighten the loop.\n\n```bragi-edit path=a.rs lines=2-3\nTWO\n```\n\nAnd add a header.\n\n```bragi-edit path=a.rs after=0\n// hi\n```\n";
        let edits = parse_edits(reply, &CodeTask::Suggest).unwrap();
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].span, Span::Lines(LineRange { start: 2, end: 3 }));
        assert_eq!(edits[1].span, Span::After { line: 0 });
        assert_eq!(explanation(reply), "Tighten the loop.\n\n\nAnd add a header.");
        let diffs = proposals(Path::new("/nonexistent"), &[file("a.rs", "one\ntwo\nthree\nfour\n")], &edits).unwrap();
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].new, "// hi\none\nTWO\nfour\n");
        assert_eq!(diffs[0].apply(&vec![true; diffs[0].hunks.len()]), diffs[0].new);
    }

    #[test]
    fn rewrite_falls_back_to_a_plain_block_and_strips_numbers() {
        let task = CodeTask::Rewrite(parse_target("a.rs:2").unwrap());
        let edits = parse_edits("<think>```x```</think>\nHere:\n```rust\n2| TWO\n```\n", &task).unwrap();
        assert_eq!(edits, vec![ProposedEdit { path: "a.rs".into(), span: Span::Lines(LineRange { start: 2, end: 2 }), text: "TWO\n".into() }]);
    }

    #[test]
    fn refuses_paths_outside_and_overlaps() {
        let e = |path: &str, span| ProposedEdit { path: path.into(), span, text: "x\n".into() };
        for p in ["../x", "/etc/passwd", "a/../../b", ".env", r"C:\x"] {
            assert!(proposals(Path::new("."), &[], &[e(p, Span::Whole)]).is_err(), "{p}");
        }
        let a = e("a", Span::Lines(LineRange { start: 1, end: 2 }));
        let b = e("a", Span::Lines(LineRange { start: 2, end: 3 }));
        assert!(apply_edits("1\n2\n3\n", &[&a, &b]).is_err());
    }

    #[test]
    fn keeps_a_missing_final_newline() {
        let e = ProposedEdit { path: "a".into(), span: Span::Lines(LineRange { start: 2, end: 2 }), text: "B\n".into() };
        assert_eq!(apply_edits("a\nb", &[&e]).unwrap(), "a\nB");
    }

    #[test]
    fn creates_a_new_file_only_whole() {
        let d = tempfile::tempdir().unwrap();
        let whole = ProposedEdit { path: "new.rs".into(), span: Span::Whole, text: "fn x() {}\n".into() };
        let diffs = proposals(d.path(), &[], std::slice::from_ref(&whole)).unwrap();
        assert!(diffs[0].created);
        assert!(diffs[0].unified().starts_with("--- /dev/null\n"));
        let lines = ProposedEdit { span: Span::Lines(LineRange { start: 1, end: 1 }), ..whole };
        assert!(proposals(d.path(), &[], &[lines]).is_err());
    }
}

#[cfg(test)]
mod demo_tests {
    use super::*;
    use crate::demo::{WARM_MODEL, answer_for};

    /// The demo node answers code requests with well-formed edits, so shells can show the whole
    /// flow without a real model.
    #[test]
    fn the_demo_answers_rewrites_and_suggestions() {
        let files = vec![ContextFile { path: "a.rs".into(), text: "fn a() {\n    one();\n    two();\n}\n".into() }];
        let task = CodeTask::Rewrite(parse_target("a.rs:2-3").unwrap());
        let req = CodeRequest { task: task.clone(), instruction: "say hi".into(), files: files.clone() };
        let reply = answer_for(WARM_MODEL, &build(&req).unwrap()[1].content);
        let edits = parse_edits(&reply, &task).unwrap();
        let diffs = proposals(Path::new("."), &files, &edits).unwrap();
        assert_eq!(diffs[0].new, format!("fn a() {{\n    // {WARM_MODEL}: say hi\n    one();\n    two();\n}}\n"));
        let req = CodeRequest { task: CodeTask::Suggest, instruction: "tidy".into(), files: files.clone() };
        let reply = answer_for(WARM_MODEL, &build(&req).unwrap()[1].content);
        let edits = parse_edits(&reply, &CodeTask::Suggest).unwrap();
        assert_eq!(edits[0].span, Span::After { line: 0 });
        assert!(!explanation(&reply).contains("bragi-edit"));
        let whole = CodeTask::Rewrite(parse_target("a.rs").unwrap());
        let req = CodeRequest { task: whole.clone(), instruction: "x".into(), files: files.clone() };
        let edits = parse_edits(&answer_for(WARM_MODEL, &build(&req).unwrap()[1].content), &whole).unwrap();
        assert_eq!(proposals(Path::new("."), &files, &edits).unwrap()[0].new, format!("// {WARM_MODEL}: x\n{}", files[0].text));
    }
}
