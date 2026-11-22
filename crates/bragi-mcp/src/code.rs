//! The code tools: suggestions and rewrites over files in the project root, answered as unified
//! diffs, and (with `--allow-writes`) applying them through the undo journal. The prompts, the
//! answer parsing, the diffs and the journal are `bragi_core::code`, the same code the terminal
//! app and the editor extension use.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;

use bragi_core::code::prompt::{self, explanation, parse_edits, proposals};
use bragi_core::code::{CodeRequest, CodeTask, FileChange, FileDiff, Journal, Limits, gather, parse_target};
use bragi_core::{CoreError, ErrorKind};
use serde_json::{Value, json};
use weirdnet_mcp::{CallToolResult, Content, Param, ParamType as T, ToolSpec};

use crate::collect;
use crate::tools::{BragiTools, bad, spec};

/// Proposals kept for `bragi_code_apply`, newest last.
pub const KEPT_PROPOSALS: usize = 32;

/// A suggestion or rewrite waiting to be applied.
#[derive(Debug, Clone)]
pub(crate) struct Proposal {
    id: String,
    diffs: Vec<FileDiff>,
}

/// The proposals of one server.
#[derive(Debug, Default)]
pub(crate) struct Proposals(Mutex<VecDeque<Proposal>>);

impl Proposals {
    fn keep(&self, p: Proposal) {
        let mut q = self.0.lock().unwrap_or_else(|e| e.into_inner());
        q.push_back(p);
        while q.len() > KEPT_PROPOSALS {
            q.pop_front();
        }
    }

    fn get(&self, id: &str) -> Option<Proposal> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|p| p.id == id).cloned()
    }
}

const ROUTE: [Param; 2] = [
    Param::optional("model", T::String, "A model id from bragi_models_list. Default: the pool's pick, or the endpoint's model."),
    Param::optional(
        "endpoint",
        T::String,
        "`pool` or an endpoint id from bragi_endpoints_list. Default: the pool (or the first endpoint when not linked).",
    ),
];

const SUGGEST_PARAMS: &[Param] = &[
    Param::optional(
        "paths",
        T::StringArray,
        "Files and folders inside the project root to review (folders respect .gitignore; binaries, secrets and big files are skipped). Default: the whole root.",
    ),
    Param::optional("instruction", T::String, "What to look for (`find bugs`, `simplify error handling`). Default: general review."),
    ROUTE[0],
    ROUTE[1],
];

const REWRITE_PARAMS: &[Param] = &[
    Param::required("target", T::String, "The file to rewrite, inside the project root, optionally with lines: `src/lib.rs`, `src/lib.rs:10-40`."),
    Param::required("instruction", T::String, "How to rewrite it."),
    Param::optional("context", T::StringArray, "More files or folders the model should see (read only)."),
    ROUTE[0],
    ROUTE[1],
];

const APPLY_PARAMS: &[Param] = &[
    Param::required("proposal", T::String, "The proposal id from bragi_code_suggest or bragi_code_rewrite."),
    Param::optional(
        "hunks",
        T::Json(r#"{"type":"object","additionalProperties":{"type":"array","items":{"type":"integer","minimum":1}}}"#),
        "Only these hunks: {\"path\": [1, 3]} (1-based, in the order the diff shows them). Files left out are not written. Default: every hunk of every file.",
    ),
];

const UNDO_PARAMS: &[Param] = &[
    Param::optional("id", T::String, "The apply id bragi_code_apply returned. Default: the last apply."),
    Param::optional("force", T::Boolean, "Undo even when a file changed since the apply (default false: refuse)."),
];

/// The code tools every server offers (they read files and change nothing on disk).
pub const CODE_TOOLS: &[ToolSpec] = &[
    spec(
        "bragi_code_suggest",
        "Ask a model to review files in the project and propose changes. Returns a unified diff, the model's explanation and a proposal id. Writes nothing; kept out of chat history.",
        SUGGEST_PARAMS,
        true,
        false,
    ),
    spec(
        "bragi_code_rewrite",
        "Ask a model to rewrite a file or a line range as instructed. Returns a unified diff, the explanation and a proposal id. Writes nothing; kept out of chat history.",
        REWRITE_PARAMS,
        true,
        false,
    ),
];

/// Offered only with `--allow-writes`.
pub const CODE_WRITE_TOOLS: &[ToolSpec] = &[
    spec(
        "bragi_code_apply",
        "WRITES a proposal (or chosen hunks of it) into the project files. Refuses when a file changed since the proposal; every apply can be undone with bragi_code_undo.",
        APPLY_PARAMS,
        false,
        true,
    ),
    spec("bragi_code_undo", "WRITES the project files back to what they were before an apply.", UNDO_PARAMS, false, true),
];

impl BragiTools {
    fn journal(&self) -> Journal {
        Journal::new(self.options().journal.clone())
    }

    async fn propose(&self, req: CodeRequest, args: &Value, skipped: Vec<(String, String)>) -> Result<CallToolResult, CoreError> {
        let messages = prompt::build(&req)?;
        let model = weirdnet_mcp::args::opt_string(args, "model").filter(|m| !m.is_empty()).map(str::to_string);
        let target = self.target_arg(args, model.as_deref())?;
        let handle = self.core().complete_to(messages, model, target)?;
        let answer = collect::answer(self.core(), handle, self.options().timeout).await?;
        let edits = parse_edits(&answer.text, &req.task)?;
        let diffs: Vec<FileDiff> = proposals(self.root(), &req.files, &edits)?.into_iter().filter(|d| !d.is_empty()).collect();
        let why = explanation(&answer.text);
        let patch = bragi_core::code::diff::unified_all(&diffs);
        let id = crate::tools::random_id("p");
        let files: Vec<Value> = diffs
            .iter()
            .map(|d| {
                let (added, removed) = d.stats();
                json!({"path": d.path, "created": d.created, "hunks": d.hunks.len(), "added": added, "removed": removed})
            })
            .collect();
        let structured = json!({
            "proposal": if diffs.is_empty() { Value::Null } else { json!(id) },
            "title": prompt::title(&req),
            "explanation": why,
            "diff": patch,
            "files": files,
            "skipped": skipped.iter().map(|(p, why)| json!({"path": p, "why": why})).collect::<Vec<_>>(),
            "model": answer.model,
        });
        let text = if diffs.is_empty() {
            format!("No change proposed.\n\n{why}")
        } else {
            self.proposals().keep(Proposal { id: id.clone(), diffs });
            format!("{why}\n\nProposal {id}:\n\n```diff\n{patch}```")
        };
        Ok(CallToolResult::new(vec![Content::text(text)]).with_structured(structured))
    }

    pub(crate) async fn code_suggest(&self, args: &Value) -> Result<CallToolResult, CoreError> {
        let paths = weirdnet_mcp::args::strings(args, "paths").unwrap_or_default();
        let targets =
            if paths.is_empty() { vec![self.root().to_path_buf()] } else { paths.iter().map(|p| self.resolve(p)).collect::<Result<_, _>>()? };
        let gathered = gather(self.root(), &targets, Limits::default())?;
        if gathered.files.is_empty() {
            return Err(bad("no readable text files there to review"));
        }
        let instruction = weirdnet_mcp::args::opt_string(args, "instruction").unwrap_or_default().to_string();
        let req = CodeRequest { task: CodeTask::Suggest, instruction, files: gathered.files };
        self.propose(req, args, gathered.skipped).await
    }

    pub(crate) async fn code_rewrite(&self, args: &Value) -> Result<CallToolResult, CoreError> {
        let target = parse_target(weirdnet_mcp::args::string(args, "target").map_err(bad)?)?;
        let instruction = weirdnet_mcp::args::string(args, "instruction").map_err(bad)?.to_string();
        let file = self.resolve(&target.path)?;
        let mut targets = vec![file.clone()];
        for c in weirdnet_mcp::args::strings(args, "context").unwrap_or_default() {
            targets.push(self.resolve(&c)?);
        }
        let gathered = gather(self.root(), &targets, Limits::default())?;
        // The target as the context names it (relative to the root, `/` separators).
        let rel = bragi_core::code::context::relative(self.root(), &file);
        if gathered.get(&rel).is_none() {
            return Err(bad(format!("{} can't be rewritten (not a text file, a secret, or too big)", target.path)));
        }
        let task = CodeTask::Rewrite(bragi_core::code::Target { path: rel, range: target.range });
        let req = CodeRequest { task, instruction, files: gathered.files };
        self.propose(req, args, gathered.skipped).await
    }

    pub(crate) fn code_apply(&self, args: &Value) -> Result<CallToolResult, CoreError> {
        let id = weirdnet_mcp::args::string(args, "proposal").map_err(bad)?;
        let p = self.proposals().get(id).ok_or_else(|| {
            CoreError::new(ErrorKind::NotFound, format!("no proposal `{id}` (the server keeps the last {KEPT_PROPOSALS}); ask again"))
        })?;
        let chosen = match args.get("hunks") {
            None | Some(Value::Null) => None,
            Some(Value::Object(m)) => Some(m.clone()),
            Some(_) => return Err(bad("`hunks` must be an object: {\"path\": [1, 2]}")),
        };
        let mut changes = Vec::new();
        for d in &p.diffs {
            let accepted: Vec<bool> = match &chosen {
                None => vec![true; d.hunks.len()],
                Some(m) => match m.get(&d.path) {
                    None => continue,
                    Some(Value::Array(ns)) => {
                        let ns: Vec<u64> = ns.iter().filter_map(Value::as_u64).collect();
                        if let Some(bad_n) = ns.iter().find(|n| **n == 0 || **n as usize > d.hunks.len()) {
                            return Err(bad(format!("{} has hunks 1 to {}, not {bad_n}", d.path, d.hunks.len())));
                        }
                        (1..=d.hunks.len()).map(|i| ns.contains(&(i as u64))).collect()
                    }
                    Some(_) => return Err(bad(format!("hunks for {} must be a list of numbers", d.path))),
                },
            };
            if !accepted.iter().any(|a| *a) {
                continue;
            }
            let path = self.root().join(&d.path);
            // The model's path stays inside the root (checked when the proposal was made); check the
            // real parent again in case a link was put there since.
            if let Some(parent) = path.parent()
                && let Ok(real) = parent.canonicalize()
                && !real.starts_with(self.root())
            {
                return Err(bad(format!("{} is outside the project root", d.path)));
            }
            changes.push(FileChange { path, before: (!d.created).then(|| d.old.clone()), after: d.apply(&accepted) });
        }
        if let Some(m) = &chosen
            && let Some(unknown) = m.keys().find(|k| !p.diffs.iter().any(|d| &d.path == *k))
        {
            return Err(bad(format!("{unknown} is not in proposal {id}")));
        }
        if changes.is_empty() {
            return Err(bad("no hunk chosen; nothing to write"));
        }
        let applied = self.journal().apply(&changes)?;
        let files: Vec<String> = applied.files.iter().map(|f| self.relative(f)).collect();
        Ok(CallToolResult::json(&json!({"applied": applied.id, "files": files})))
    }

    pub(crate) fn code_undo(&self, args: &Value) -> Result<CallToolResult, CoreError> {
        let force = weirdnet_mcp::args::boolean(args, "force", false);
        let journal = self.journal();
        let undone = match weirdnet_mcp::args::opt_string(args, "id").filter(|i| !i.is_empty()) {
            Some(id) => journal.undo(id, force)?,
            None => journal.undo_last(force)?,
        };
        let files: Vec<String> = undone.files.iter().map(|f| self.relative(f)).collect();
        Ok(CallToolResult::json(&json!({"undone": undone.id, "files": files})))
    }

    fn relative(&self, p: &std::path::Path) -> String {
        bragi_core::code::context::relative(self.root(), p)
    }
}

/// Where the undo journal goes under a Bragi home (never in the project).
pub fn journal_dir(home: &std::path::Path) -> PathBuf {
    home.join("mcp").join("code-undo")
}
