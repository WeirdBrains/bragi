//! MCP prompts: ready-made requests a client fills in and sends to its own model. Files are read
//! the way the code tools read them (inside the project root, through `bragi_core::code::gather`,
//! so secrets, binaries and oversize files are refused), and sessions come from Bragi's history.

use bragi_core::code::prompt::{self as code_prompt};
use bragi_core::code::{CodeRequest, CodeTask, ContextFile, Limits, PromptRole, gather, parse_target};
use bragi_core::{CoreError, ErrorKind, MessageState, Role};
use serde_json::Value;
use weirdnet_mcp::{GetPromptResult, PromptArg, PromptError, PromptMessage, PromptSpec};

use crate::tools::BragiTools;

/// Messages of a session `bragi_continue_session` replays at most.
pub const CONTINUE_MESSAGES: usize = 40;

const PATH: PromptArg = PromptArg::required("path", "A file inside the project root, optionally with lines: `src/lib.rs` or `src/lib.rs:10-40`.");
const SESSION: PromptArg = PromptArg::required("session", "A session id (bragi_sessions_list, or bragi://sessions).");

/// Every prompt the server offers.
pub const PROMPTS: &[PromptSpec] = &[
    PromptSpec::new(
        "bragi_review_file",
        "Review a file (or some lines of it) from the project for bugs, clarity and simplifications.",
        &[PATH, PromptArg::optional("focus", "What to look for, such as `error handling` (default: a general review).")],
    )
    .with_title("Review a file"),
    PromptSpec::new(
        "bragi_rewrite",
        "Rewrite a file or a line range as instructed, answering with `bragi-edit` blocks (the format Bragi's code tools apply).",
        &[PATH, PromptArg::required("instruction", "How to rewrite it.")],
    )
    .with_title("Rewrite code"),
    PromptSpec::new(
        "bragi_explain",
        "Explain what some code does: a file or lines of it from the project, or a pasted selection.",
        &[
            PromptArg::optional("path", "A file inside the project root, optionally with lines (`src/lib.rs:10-40`). Give this or `selection`."),
            PromptArg::optional("selection", "The code itself. Give this or `path`."),
            PromptArg::optional("language", "The selection's language, for the code fence."),
        ],
    )
    .with_title("Explain code"),
    PromptSpec::new("bragi_session_summary", "Summarize a Bragi session: what was asked, what was decided, what is still open.", &[SESSION])
        .with_title("Summarize a session"),
    PromptSpec::new(
        "bragi_continue_session",
        "Pick up a Bragi session: its recent messages as the conversation so far, then your next message.",
        &[SESSION, PromptArg::optional("prompt", "The next message (default: ask to carry on from where it stopped).")],
    )
    .with_title("Continue a session"),
];

fn invalid(e: CoreError) -> PromptError {
    match e.kind {
        ErrorKind::BadRequest | ErrorKind::NotFound => PromptError::InvalidArguments(e.message),
        _ => PromptError::Failed(e.message),
    }
}

fn arg<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args[k].as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn fence(text: &str, lang: &str) -> String {
    let ticks = "`".repeat(text.split(|c| c != '`').map(str::len).max().unwrap_or(0).max(2) + 1);
    let nl = if text.ends_with('\n') { "" } else { "\n" };
    format!("{ticks}{lang}\n{text}{nl}{ticks}")
}

fn lang_of(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, e)| e) {
        Some("rs") => "rust",
        Some("py") => "python",
        Some("js" | "mjs") => "javascript",
        Some("ts") => "typescript",
        Some("dart") => "dart",
        Some("go") => "go",
        Some("toml") => "toml",
        Some("md") => "markdown",
        Some("sh") => "sh",
        Some("json") => "json",
        Some("c" | "h") => "c",
        Some("cpp" | "cc" | "hpp") => "cpp",
        _ => "",
    }
}

impl BragiTools {
    /// One file from the project, as the code tools read it, and the lines asked for (`None`: all).
    fn prompt_file(&self, spec: &str) -> Result<(ContextFile, Option<(usize, usize)>), CoreError> {
        let target = parse_target(spec)?;
        let file = self.resolve(&target.path)?;
        let gathered = gather(self.root(), std::slice::from_ref(&file), Limits::default())?;
        let rel = bragi_core::code::context::relative(self.root(), &file);
        let Some(f) = gathered.files.into_iter().find(|f| f.path == rel) else {
            let why = gathered.skipped.into_iter().find(|(p, _)| *p == rel).map(|(_, w)| w).unwrap_or_else(|| "not a readable text file".into());
            return Err(CoreError::new(ErrorKind::BadRequest, format!("{} can't be used: {why}", target.path)));
        };
        let range = match target.range {
            None => None,
            Some(r) => {
                let n = f.text.lines().count();
                if r.end > n.max(1) {
                    return Err(CoreError::new(ErrorKind::BadRequest, format!("{} has {n} lines; {r} is past the end", target.path)));
                }
                Some((r.start, r.end))
            }
        };
        Ok((f, range))
    }

    fn excerpt(f: &ContextFile, range: Option<(usize, usize)>) -> (String, String) {
        match range {
            None => (f.path.clone(), fence(&f.text, lang_of(&f.path))),
            Some((a, b)) => {
                let lines: Vec<&str> = f.text.lines().skip(a - 1).take(b + 1 - a).collect();
                (format!("lines {a}-{b} of {}", f.path), fence(&lines.join("\n"), lang_of(&f.path)))
            }
        }
    }

    pub(crate) async fn prompt(&self, spec: &PromptSpec, args: &Value) -> Result<GetPromptResult, PromptError> {
        match spec.name {
            "bragi_review_file" => {
                let (f, range) = self.prompt_file(arg(args, "path").unwrap_or_default()).map_err(invalid)?;
                let (what, code) = Self::excerpt(&f, range);
                let focus = arg(args, "focus").map(|x| format!(", looking especially at {x}")).unwrap_or_default();
                let text = format!(
                    "Review {what}{focus}. Point at the lines you mean and suggest concrete changes (fixes, clarity, simplifications); skip changes that are only taste.\n\n{code}"
                );
                Ok(GetPromptResult::new(vec![PromptMessage::user(text)]).with_description(format!("Review {what}")))
            }
            "bragi_rewrite" => {
                let path = arg(args, "path").unwrap_or_default();
                let instruction = arg(args, "instruction").ok_or_else(|| PromptError::InvalidArguments("the instruction is empty".into()))?;
                let (f, _) = self.prompt_file(path).map_err(invalid)?;
                let target = parse_target(path).map_err(invalid)?;
                let task = CodeTask::Rewrite(bragi_core::code::Target { path: f.path.clone(), range: target.range });
                let req = CodeRequest { task, instruction: instruction.to_string(), files: vec![f] };
                let title = code_prompt::title(&req);
                // The same messages Bragi's own rewrite sends; MCP prompts have no system role, so
                // the system text leads the first user message.
                let mut system = String::new();
                let mut out = Vec::new();
                for m in code_prompt::build(&req).map_err(invalid)? {
                    match m.role {
                        PromptRole::System => system.push_str(&format!("{}\n\n", m.content)),
                        PromptRole::User => out.push(PromptMessage::user(format!("{}{}", std::mem::take(&mut system), m.content))),
                        PromptRole::Assistant => out.push(PromptMessage::assistant(m.content)),
                    }
                }
                Ok(GetPromptResult::new(out).with_description(title))
            }
            "bragi_explain" => {
                let (what, code) = match (arg(args, "path"), args["selection"].as_str().filter(|s| !s.trim().is_empty())) {
                    (Some(_), Some(_)) => return Err(PromptError::InvalidArguments("give `path` or `selection`, not both".into())),
                    (None, None) => {
                        return Err(PromptError::InvalidArguments("give `path` (a file in the project) or `selection` (the code)".into()));
                    }
                    (Some(p), None) => {
                        let (f, range) = self.prompt_file(p).map_err(invalid)?;
                        Self::excerpt(&f, range)
                    }
                    (None, Some(sel)) => ("this code".to_string(), fence(sel, arg(args, "language").unwrap_or(""))),
                };
                let text = format!(
                    "Explain {what}: what it does, how, and anything surprising (edge cases, side effects, likely bugs). Be concrete and refer to names in the code.\n\n{code}"
                );
                Ok(GetPromptResult::new(vec![PromptMessage::user(text)]).with_description(format!("Explain {what}")))
            }
            "bragi_session_summary" => {
                let id = arg(args, "session").unwrap_or_default();
                let c = self.core().conversation(id).map_err(invalid)?;
                let text = format!(
                    "Summarize this conversation in a few bullet points: what was asked, what was answered or decided, and what is still open.\n\n{}",
                    crate::tools::transcript(&c, None)
                );
                Ok(GetPromptResult::new(vec![PromptMessage::user(text)]).with_description(format!("Summarize “{}”", c.title)))
            }
            "bragi_continue_session" => {
                let id = arg(args, "session").unwrap_or_default();
                let c = self.core().conversation(id).map_err(invalid)?;
                let skip = c.messages.len().saturating_sub(CONTINUE_MESSAGES);
                let mut out = Vec::new();
                for m in c.messages.iter().skip(skip).filter(|m| !m.text.trim().is_empty()) {
                    let mut text = m.text.clone();
                    for f in &m.attachments {
                        text.push_str(&format!("\n\n(attached: {})", f.label()));
                    }
                    if m.role == Role::Assistant && m.state != MessageState::Complete {
                        text.push_str("\n\n(this answer was cut off)");
                    }
                    out.push(match m.role {
                        Role::User => PromptMessage::user(text),
                        Role::Assistant => PromptMessage::assistant(text),
                    });
                }
                let next = arg(args, "prompt").unwrap_or("Carry on from where this conversation stopped.");
                out.push(PromptMessage::user(next.to_string()));
                Ok(GetPromptResult::new(out).with_description(format!("Continue “{}”", c.title)))
            }
            _ => Err(PromptError::NotFound),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prompt_table_is_valid() {
        PromptSpec::check(PROMPTS).unwrap();
    }

    #[test]
    fn fences_outlast_the_code() {
        assert_eq!(fence("a", "rust"), "```rust\na\n```");
        assert!(fence("x\n```\ny", "").starts_with("````\n"));
    }
}
