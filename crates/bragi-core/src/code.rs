//! Code suggestions and rewrites, shared by every shell that edits code (the editor extension
//! through `bragi serve`, and the TUI): the prompts built from an editor's context, reading a
//! model's answer back as code, and the line diff a shell previews before the user accepts.
//!
//! The context carries text, not offsets: `before`, `selection` and `after` concatenate to the
//! file, so a shell never has to agree with the core on how to count characters (VS Code counts
//! UTF-16 units). An empty selection is the cursor.

use serde::{Deserialize, Serialize};

use crate::attach::Attachment;
use crate::error::{CoreError, ErrorKind, Result};

/// Text before the cursor a suggestion sees.
pub const SUGGEST_BEFORE_CHARS: usize = 6_000;
/// Text after the cursor a suggestion sees.
pub const SUGGEST_AFTER_CHARS: usize = 2_000;
/// The most of the current file a rewrite or a chat sees.
pub const MAX_FILE_CHARS: usize = 48_000;
/// The most of each other open file a rewrite or a chat sees.
pub const MAX_OPEN_FILE_CHARS: usize = 8_000;
/// Other open files sent along (a chat's attachments also hold the current file and the diagnostics).
pub const MAX_OPEN_FILES: usize = 3;
pub const MAX_DIAGNOSTICS: usize = 40;

/// What the editor shows: the current file split around the selection, plus what else is open.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorContext {
    /// The file's path, relative to the workspace when it has one.
    pub path: String,
    /// The editor's language id (`rust`, `typescript`, …).
    pub language: String,
    pub before: String,
    /// The selected text; empty means the cursor sits between `before` and `after`.
    pub selection: String,
    pub after: String,
    pub open_files: Vec<OpenFile>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenFile {
    pub path: String,
    pub language: String,
    pub text: String,
}

/// A problem the editor reports in the current file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Diagnostic {
    /// 1-based.
    pub line: u32,
    /// `error`, `warning`, `info` or `hint`.
    pub severity: String,
    pub message: String,
    pub source: Option<String>,
}

/// What a rewrite replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// The selection (the whole file when nothing is selected).
    #[default]
    Selection,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptRole {
    System,
    User,
    Assistant,
}

/// One message of a prompt, independent of the transport (Weirdnet pool or an OpenAI endpoint).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptMessage {
    pub role: PromptRole,
    pub content: String,
}

impl PromptMessage {
    pub fn system(c: impl Into<String>) -> Self {
        Self { role: PromptRole::System, content: c.into() }
    }
    pub fn user(c: impl Into<String>) -> Self {
        Self { role: PromptRole::User, content: c.into() }
    }
}

impl EditorContext {
    pub fn file(&self) -> String {
        format!("{}{}{}", self.before, self.selection, self.after)
    }

    /// The scope a rewrite really has: a selection rewrite with nothing selected is a file one.
    pub fn effective_scope(&self, scope: Scope) -> Scope {
        if scope == Scope::Selection && self.selection.is_empty() { Scope::File } else { scope }
    }

    /// The text a rewrite in `scope` replaces.
    pub fn target(&self, scope: Scope) -> String {
        match self.effective_scope(scope) {
            Scope::Selection => self.selection.clone(),
            Scope::File => self.file(),
        }
    }

    /// The file once `replacement` takes the place of the text in `scope`.
    pub fn apply(&self, scope: Scope, replacement: &str) -> String {
        match self.effective_scope(scope) {
            Scope::Selection => format!("{}{}{}", self.before, replacement, self.after),
            Scope::File => replacement.to_string(),
        }
    }

    fn name(&self) -> &str {
        if self.path.is_empty() { "untitled" } else { &self.path }
    }
}

fn bad(msg: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::BadRequest, msg)
}

/// The last `n` characters of `s`, cut at a line start when one is near.
fn tail(s: &str, n: usize) -> &str {
    let count = s.chars().count();
    if count <= n {
        return s;
    }
    let start = s.char_indices().nth(count - n).map(|(i, _)| i).unwrap_or(0);
    let cut = &s[start..];
    match cut.find('\n') {
        Some(i) if i < 200 => &cut[i + 1..],
        _ => cut,
    }
}

/// The first `n` characters of `s`, cut at a line end when one is near.
fn head(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        None => s,
        Some((end, _)) => {
            let cut = &s[..end];
            match cut.rfind('\n') {
                Some(i) if end - i < 200 => &cut[..=i],
                _ => cut,
            }
        }
    }
}

/// A fence longer than any run of backticks in `text`.
fn fence_for(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    "`".repeat(longest.max(2) + 1)
}

fn fenced(lang: &str, text: &str) -> String {
    let f = fence_for(text);
    format!("{f}{lang}\n{text}{}{f}", if text.ends_with('\n') || text.is_empty() { "" } else { "\n" })
}

fn diagnostics_text(ctx: &EditorContext) -> String {
    ctx.diagnostics
        .iter()
        .take(MAX_DIAGNOSTICS)
        .map(|d| {
            let src = d.source.as_deref().map(|s| format!(" [{s}]")).unwrap_or_default();
            format!("line {}: {}{src}: {}", d.line, if d.severity.is_empty() { "problem" } else { &d.severity }, d.message.trim())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn open_files_text(ctx: &EditorContext) -> String {
    let mut out = String::new();
    for f in ctx.open_files.iter().filter(|f| f.path != ctx.path && !f.text.is_empty()).take(MAX_OPEN_FILES) {
        out.push_str(&format!("Open file {}:\n{}\n\n", f.path, fenced(&f.language, head(&f.text, MAX_OPEN_FILE_CHARS))));
    }
    out
}

const SUGGEST_SYSTEM: &str = "You are a code completion engine. You get the code before and after the cursor and answer \
with only the text to insert at the cursor: no explanation, no repetition of the surrounding code, no Markdown fences. \
Keep it short (one statement or block) and match the file's style and indentation. Answer with nothing if nothing fits.";

/// The prompt for an inline suggestion at the cursor (the end of the selection, when there is one).
pub fn suggest_prompt(ctx: &EditorContext) -> Vec<PromptMessage> {
    let before = format!("{}{}", ctx.before, ctx.selection);
    let (before, after) = (tail(&before, SUGGEST_BEFORE_CHARS), head(&ctx.after, SUGGEST_AFTER_CHARS));
    let mut user = format!("File: {} ({})\n", ctx.name(), if ctx.language.is_empty() { "plain text" } else { &ctx.language });
    let diags = diagnostics_text(ctx);
    if !diags.is_empty() {
        user.push_str(&format!("Problems in this file:\n{diags}\n"));
    }
    user.push_str(&format!("\n<code_before_cursor>\n{before}</code_before_cursor>\n<code_after_cursor>\n{after}</code_after_cursor>\n"));
    user.push_str("\nWrite only the text that goes at the cursor.");
    vec![PromptMessage::system(SUGGEST_SYSTEM), PromptMessage::user(user)]
}

const REWRITE_SYSTEM: &str = "You rewrite code as asked. Answer with the complete replacement in one fenced code block \
and nothing else: keep everything you were not asked to change, keep the style and indentation, and never leave \
placeholders such as \"rest unchanged\".";

/// The prompt for rewriting the selection (or the file) as `instruction` says.
pub fn rewrite_prompt(ctx: &EditorContext, instruction: &str, scope: Scope) -> Result<Vec<PromptMessage>> {
    let instruction = instruction.trim();
    if instruction.is_empty() {
        return Err(bad("say how to rewrite the code"));
    }
    let scope = ctx.effective_scope(scope);
    let file = ctx.file();
    if file.chars().count() > MAX_FILE_CHARS && scope == Scope::File {
        return Err(bad(format!("the file is over {MAX_FILE_CHARS} characters; select the part to rewrite")));
    }
    let lang = &ctx.language;
    let mut user = open_files_text(ctx);
    let diags = diagnostics_text(ctx);
    match scope {
        Scope::File => {
            user.push_str(&format!("The file {}:\n{}\n\n", ctx.name(), fenced(lang, &file)));
        }
        Scope::Selection => {
            let before = tail(&ctx.before, MAX_FILE_CHARS / 2);
            let after = head(&ctx.after, MAX_FILE_CHARS / 4);
            user.push_str(&format!(
                "The file {}, with the part to rewrite between <selection> and </selection>:\n{}\n\n",
                ctx.name(),
                fenced(lang, &format!("{before}<selection>{}</selection>{after}", ctx.selection))
            ));
        }
    }
    if !diags.is_empty() {
        user.push_str(&format!("Problems the editor reports in this file:\n{diags}\n\n"));
    }
    let what = if scope == Scope::File { "the whole file" } else { "only the selected part (without the <selection> tags)" };
    user.push_str(&format!("Rewrite {what}: {instruction}\nAnswer with the replacement in one fenced code block."));
    Ok(vec![PromptMessage::system(REWRITE_SYSTEM), PromptMessage::user(user)])
}

/// A chat's editor context as attachments (the current file or selection, other open files,
/// the diagnostics), at most [`crate::attach::MAX_PER_MESSAGE`].
pub fn chat_attachments(ctx: &EditorContext) -> Vec<Attachment> {
    let mut out = Vec::new();
    let file = ctx.file();
    if !file.is_empty() {
        out.push(Attachment { name: ctx.name().to_string(), text: head(&file, MAX_FILE_CHARS).to_string() });
    }
    if !ctx.selection.is_empty() {
        out.push(Attachment { name: format!("selection in {}", ctx.name()), text: head(&ctx.selection, MAX_FILE_CHARS).to_string() });
    }
    let diags = diagnostics_text(ctx);
    if !diags.is_empty() {
        out.push(Attachment { name: format!("problems in {}", ctx.name()), text: diags });
    }
    for f in ctx.open_files.iter().filter(|f| f.path != ctx.path && !f.text.is_empty()) {
        if out.len() >= crate::attach::MAX_PER_MESSAGE {
            break;
        }
        out.push(Attachment { name: f.path.clone(), text: head(&f.text, MAX_OPEN_FILE_CHARS).to_string() });
    }
    for a in &mut out {
        a.name = a.name.replace(['/', '\\'], "∕");
    }
    out
}

/// Drops a reasoning model's `<think>…</think>` preamble.
fn strip_thinking(answer: &str) -> &str {
    match (answer.find("<think>"), answer.find("</think>")) {
        (Some(s), Some(e)) if s < e && answer[..s].trim().is_empty() => &answer[e + "</think>".len()..],
        (None, Some(e)) => &answer[e + "</think>".len()..],
        _ => answer,
    }
}

/// The code in a model's answer: the first fenced block's content when there is one (its fence
/// may be longer than three backticks), else the whole answer, without reasoning.
pub fn extract_code(answer: &str) -> String {
    let answer = strip_thinking(answer);
    let lines: Vec<&str> = answer.split_inclusive('\n').collect();
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim_start();
        let ticks = t.chars().take_while(|&c| c == '`').count();
        if ticks < 3 {
            continue;
        }
        let fence = &t[..ticks];
        let mut body = String::new();
        for l in &lines[i + 1..] {
            let t = l.trim();
            if t.starts_with(fence) && t.trim_start_matches('`').is_empty() {
                return body;
            }
            body.push_str(l);
        }
        // No closing fence (a cut-off answer): everything after the opening one.
        return body;
    }
    answer.trim_matches('\n').to_string()
}

/// A rewrite's replacement, keeping the original's trailing newline (or lack of one).
pub fn rewrite_result(answer: &str, original: &str) -> String {
    let mut code = extract_code(answer);
    let had = original.ends_with('\n');
    while code.ends_with('\n') {
        code.pop();
    }
    if had {
        code.push('\n');
    }
    code
}

/// A suggestion as it goes in: the code, minus any repeat of the line before the cursor at its
/// start and of the text after the cursor at its end.
pub fn clean_suggestion(answer: &str, ctx: &EditorContext) -> String {
    let mut s = extract_code(answer);
    let before = format!("{}{}", ctx.before, ctx.selection);
    let line = before.rsplit('\n').next().unwrap_or("");
    if !line.trim().is_empty() && s.starts_with(line) {
        s = s[line.len()..].to_string();
    }
    // The model repeated what follows the cursor: keep only what is new.
    let after = ctx.after.trim_start();
    if !after.is_empty() {
        let first = after.lines().next().unwrap_or("").trim();
        if !first.is_empty()
            && let Some(i) = s.rfind(first)
            && s[i..].trim_end().len() <= after.len()
            && after.starts_with(s[i..].trim_end())
        {
            s.truncate(i);
        }
    }
    let s = s.trim_end().to_string();
    if s.trim().is_empty() { String::new() } else { s }
}

/// One line of a diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", content = "text", rename_all = "snake_case")]
pub enum DiffLine {
    Same(String),
    Add(String),
    Del(String),
}

/// Most lines diffed line by line between the common start and end; past it the middle is
/// shown as replaced whole.
const DIFF_CELLS: usize = 4_000_000;

/// The line diff from `old` to `new` (longest common subsequence after trimming the common
/// start and end). Lines keep no newline.
pub fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suf = a[pre..].iter().rev().zip(b[pre..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (am, bm) = (&a[pre..a.len() - suf], &b[pre..b.len() - suf]);
    let mut out: Vec<DiffLine> = a[..pre].iter().map(|l| DiffLine::Same(l.to_string())).collect();
    if am.len().saturating_mul(bm.len()) > DIFF_CELLS {
        out.extend(am.iter().map(|l| DiffLine::Del(l.to_string())));
        out.extend(bm.iter().map(|l| DiffLine::Add(l.to_string())));
    } else {
        let (n, m) = (am.len(), bm.len());
        // lcs[i][j]: the LCS of am[i..] and bm[j..].
        let mut lcs = vec![0u32; (n + 1) * (m + 1)];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[i * (m + 1) + j] =
                    if am[i] == bm[j] { lcs[(i + 1) * (m + 1) + j + 1] + 1 } else { lcs[(i + 1) * (m + 1) + j].max(lcs[i * (m + 1) + j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && am[i] == bm[j] {
                out.push(DiffLine::Same(am[i].to_string()));
                i += 1;
                j += 1;
            } else if i < n && (j == m || lcs[(i + 1) * (m + 1) + j] >= lcs[i * (m + 1) + j + 1]) {
                // Removals before additions, as unified diffs show them.
                out.push(DiffLine::Del(am[i].to_string()));
                i += 1;
            } else {
                out.push(DiffLine::Add(bm[j].to_string()));
                j += 1;
            }
        }
    }
    out.extend(a[a.len() - suf..].iter().map(|l| DiffLine::Same(l.to_string())));
    out
}

/// Lines added and removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DiffStats {
    pub added: usize,
    pub removed: usize,
}

pub fn diff_stats(diff: &[DiffLine]) -> DiffStats {
    let mut s = DiffStats::default();
    for l in diff {
        match l {
            DiffLine::Add(_) => s.added += 1,
            DiffLine::Del(_) => s.removed += 1,
            DiffLine::Same(_) => {}
        }
    }
    s
}

/// A unified diff (`---`/`+++` headers, `@@` hunks with `context` lines around each change);
/// empty when nothing changed.
pub fn unified_diff(old: &str, new: &str, path: &str, context: usize) -> String {
    let diff = diff_lines(old, new);
    if diff.iter().all(|l| matches!(l, DiffLine::Same(_))) {
        return String::new();
    }
    // Which lines a hunk shows: every change and `context` lines either side.
    let changed: Vec<usize> = diff.iter().enumerate().filter(|(_, l)| !matches!(l, DiffLine::Same(_))).map(|(i, _)| i).collect();
    let mut show = vec![false; diff.len()];
    for &c in &changed {
        for s in show.iter_mut().take((c + context + 1).min(diff.len())).skip(c.saturating_sub(context)) {
            *s = true;
        }
    }
    let mut out = format!("--- a/{path}\n+++ b/{path}\n");
    let (mut old_line, mut new_line) = (1usize, 1usize);
    let mut i = 0;
    while i < diff.len() {
        if !show[i] {
            match diff[i] {
                DiffLine::Same(_) => {
                    old_line += 1;
                    new_line += 1;
                }
                DiffLine::Add(_) => new_line += 1,
                DiffLine::Del(_) => old_line += 1,
            }
            i += 1;
            continue;
        }
        let start = i;
        while i < diff.len() && show[i] {
            i += 1;
        }
        let hunk = &diff[start..i];
        let olds = hunk.iter().filter(|l| !matches!(l, DiffLine::Add(_))).count();
        let news = hunk.iter().filter(|l| !matches!(l, DiffLine::Del(_))).count();
        let at = |line: usize, n: usize| if n == 0 { line - 1 } else { line };
        out.push_str(&format!("@@ -{},{olds} +{},{news} @@\n", at(old_line, olds), at(new_line, news)));
        for l in hunk {
            match l {
                DiffLine::Same(t) => out.push_str(&format!(" {t}\n")),
                DiffLine::Add(t) => out.push_str(&format!("+{t}\n")),
                DiffLine::Del(t) => out.push_str(&format!("-{t}\n")),
            }
        }
        old_line += olds;
        new_line += news;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(before: &str, selection: &str, after: &str) -> EditorContext {
        EditorContext {
            path: "src/lib.rs".into(),
            language: "rust".into(),
            before: before.into(),
            selection: selection.into(),
            after: after.into(),
            ..Default::default()
        }
    }

    #[test]
    fn suggest_prompt_holds_the_code_around_the_cursor_and_the_problems() {
        let mut c = ctx("fn add(a: i32, b: i32) -> i32 {\n    ", "", "\n}\n");
        c.diagnostics.push(Diagnostic { line: 1, severity: "error".into(), message: "missing return".into(), source: Some("rustc".into()) });
        let p = suggest_prompt(&c);
        assert_eq!(p[0].role, PromptRole::System);
        assert!(p[1].content.contains("<code_before_cursor>\nfn add(a: i32, b: i32) -> i32 {\n    </code_before_cursor>"), "{}", p[1].content);
        assert!(p[1].content.contains("<code_after_cursor>\n\n}\n</code_after_cursor>"));
        assert!(p[1].content.contains("line 1: error [rustc]: missing return"));
    }

    #[test]
    fn suggest_prompt_keeps_only_a_window_of_a_big_file() {
        let big = "let x = 1;\n".repeat(5_000);
        let p = suggest_prompt(&ctx(&big, "", &big));
        assert!(p[1].content.len() < SUGGEST_BEFORE_CHARS + SUGGEST_AFTER_CHARS + 1_000);
        assert!(p[1].content.contains("<code_before_cursor>\nlet x"), "cut at a line start");
    }

    #[test]
    fn rewrite_prompts_mark_the_selection_or_send_the_file() {
        let c = ctx("a\n", "b\n", "c\n");
        let p = rewrite_prompt(&c, "make it loud", Scope::Selection).unwrap();
        assert!(p[1].content.contains("a\n<selection>b\n</selection>c\n"), "{}", p[1].content);
        assert!(p[1].content.contains("only the selected part"));
        let p = rewrite_prompt(&c, "make it loud", Scope::File).unwrap();
        assert!(p[1].content.contains("```rust\na\nb\nc\n```"));
        // Nothing selected: a selection rewrite is a file one.
        let p = rewrite_prompt(&ctx("a\n", "", "c\n"), "x", Scope::Selection).unwrap();
        assert!(p[1].content.contains("the whole file"));
        assert!(rewrite_prompt(&c, "  ", Scope::File).is_err());
    }

    #[test]
    fn open_files_ride_along_but_not_the_current_one_twice() {
        let mut c = ctx("a", "", "");
        c.open_files = vec![
            OpenFile { path: "src/lib.rs".into(), language: "rust".into(), text: "a".into() },
            OpenFile { path: "src/other.rs".into(), language: "rust".into(), text: "fn other() {}".into() },
        ];
        let p = rewrite_prompt(&c, "x", Scope::File).unwrap();
        assert_eq!(p[1].content.matches("Open file").count(), 1);
        assert!(p[1].content.contains("Open file src/other.rs"));
        let a = chat_attachments(&c);
        assert_eq!(a.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["src∕lib.rs", "src∕other.rs"]);
    }

    #[test]
    fn code_is_read_out_of_fences_and_thinking() {
        assert_eq!(extract_code("Here:\n```rust\nfn a() {}\n```\nDone."), "fn a() {}\n");
        assert_eq!(extract_code("````md\n```x```\n````"), "```x```\n");
        assert_eq!(extract_code("<think>hmm ```no```</think>\nlet a = 1;"), "let a = 1;");
        assert_eq!(extract_code("```\ncut off"), "cut off");
        assert_eq!(extract_code("\nplain\n"), "plain");
    }

    #[test]
    fn rewrites_keep_the_original_trailing_newline() {
        assert_eq!(rewrite_result("```\nx\n\n```", "y\n"), "x\n");
        assert_eq!(rewrite_result("```\nx\n```", "y"), "x");
        let c = ctx("a\n", "b\n", "c\n");
        assert_eq!(c.apply(Scope::Selection, "B\n"), "a\nB\nc\n");
        assert_eq!(c.apply(Scope::File, "z\n"), "z\n");
        assert_eq!(c.target(Scope::Selection), "b\n");
    }

    #[test]
    fn suggestions_drop_repeats_of_the_surrounding_code() {
        let c = ctx("    let total = ", "", ";\n}\n");
        assert_eq!(clean_suggestion("    let total = a + b", &c), "a + b");
        assert_eq!(clean_suggestion("a + b;\n}", &c), "a + b");
        assert_eq!(clean_suggestion("```rust\na + b\n```", &c), "a + b");
        assert_eq!(clean_suggestion("   \n", &c), "");
    }

    #[test]
    fn diffs_and_unified_hunks() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\n";
        let new = "a\nb\nC\nd\ne\nf\ng\nh\ni\nj\n";
        let d = diff_lines(old, new);
        assert_eq!(diff_stats(&d), DiffStats { added: 2, removed: 1 });
        let u = unified_diff(old, new, "x.txt", 1);
        assert_eq!(u, "--- a/x.txt\n+++ b/x.txt\n@@ -2,3 +2,3 @@\n b\n-c\n+C\n d\n@@ -9,1 +9,2 @@\n i\n+j\n");
        assert_eq!(unified_diff(old, old, "x", 3), "");
        assert_eq!(unified_diff("", "x\n", "n", 3), "--- a/n\n+++ b/n\n@@ -0,0 +1,1 @@\n+x\n");
    }

    #[test]
    fn huge_rewrites_fall_back_to_replacing_the_middle() {
        let old: String = (0..3000).map(|i| format!("o{i}\n")).collect();
        let new: String = (0..3000).map(|i| format!("n{i}\n")).collect();
        let s = diff_stats(&diff_lines(&old, &new));
        assert_eq!(s, DiffStats { added: 3000, removed: 3000 });
    }
}
