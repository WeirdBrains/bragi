//! Fitting a code request into the model's context window.
//!
//! [`context::gather`](super::context::gather) caps what is read in bytes, which says nothing
//! about whether the prompt fits the model: 215 KiB of Python is about 90k tokens, three times a
//! 32k window. Before a request goes out, [`fit`] estimates the prompt in tokens against the
//! window the model is actually served with (the node's catalog lists it; a server's
//! `exceed_context_size_error` names it too), keeps room for the answer and a safety margin, and
//! leaves out the files that don't fit. A rewrite's target is never left out: if it alone is too
//! big, the request is refused with the numbers.
//!
//! The estimate is deliberately on the high side (about 2.5 bytes per token, where code usually
//! runs 3 to 4), and [`Overflow`] reads a server's refusal so a shell can retry once with the
//! window and the ratio the server reported.

use super::context::ContextFile;
use super::prompt::{CodeRequest, CodeTask, build, file_block};
use crate::error::{CoreError, ErrorKind, Result};

/// Bytes per token the estimate assumes, as a fraction (5/2 = 2.5).
const BYTES_PER_TOKEN_NUM: usize = 5;
const BYTES_PER_TOKEN_DEN: usize = 2;
/// Tokens a chat template adds around each message.
pub const PER_MESSAGE: usize = 8;

/// A rough, high-side token count for `text`.
pub fn estimate_tokens(text: &str) -> usize {
    (text.len() * BYTES_PER_TOKEN_DEN).div_ceil(BYTES_PER_TOKEN_NUM)
}

/// How many prompt tokens a request may use on a model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TokenBudget {
    /// The model's context window as served (llama.cpp `n_ctx`, vLLM `max_model_len`).
    pub context: u32,
    /// Kept free for the answer.
    pub output: u32,
    /// Real tokens per estimated token, from a server's count (1.0 until one is known; never
    /// below 1.0, so the estimate only gets stricter).
    pub ratio: f64,
}

impl TokenBudget {
    /// The budget for a `context`-token window: a quarter of it (1k to 8k) for the answer.
    pub fn for_context(context: u32) -> Self {
        let output = (context / 4).clamp(1024.min(context / 2), 8192);
        Self { context, output, ratio: 1.0 }
    }

    /// The same window with estimates scaled by `ratio` (ignored below 1.0).
    pub fn with_ratio(self, ratio: f64) -> Self {
        Self { ratio: if ratio.is_finite() { ratio.max(1.0) } else { 1.0 }, ..self }
    }

    /// Tokens the prompt may take: the window less the answer's room and a 5% margin.
    pub fn prompt_tokens(&self) -> usize {
        let c = self.context as usize;
        c.saturating_sub(self.output as usize).saturating_sub(c / 20 + 64)
    }

    /// `n` estimated tokens as the server would likely count them.
    pub fn scaled(&self, n: usize) -> usize {
        (n as f64 * self.ratio).ceil() as usize
    }
}

/// A request trimmed to fit, and what was taken out.
#[derive(Debug, Clone, PartialEq)]
pub struct Fitted {
    pub request: CodeRequest,
    /// `(path, estimated tokens)` for every file left out, in the order they were given.
    pub dropped: Vec<(String, usize)>,
    /// The whole prompt's estimate (scaled by the budget's ratio).
    pub tokens: usize,
    pub budget: TokenBudget,
}

impl Fitted {
    /// One line for a shell: empty when nothing was left out.
    pub fn note(&self, model: &str) -> String {
        if self.dropped.is_empty() {
            return String::new();
        }
        let kept = self.request.files.len();
        let total = kept + self.dropped.len();
        let names: Vec<&str> = self.dropped.iter().take(4).map(|(p, _)| p.as_str()).collect();
        let more = if self.dropped.len() > names.len() { format!(" and {} more", self.dropped.len() - names.len()) } else { String::new() };
        format!(
            "Sent {kept} of {total} files (~{} tokens) to fit {model}'s {} context; left out {}{more}",
            thousands(self.tokens),
            thousands(self.budget.context as usize),
            names.join(", ")
        )
    }
}

/// `91723` as `91.7k`.
pub fn thousands(n: usize) -> String {
    if n < 1000 { n.to_string() } else { format!("{:.1}k", n as f64 / 1000.0).replace(".0k", "k") }
}

/// The estimated prompt tokens for `req` as [`build`] would send it.
pub fn request_tokens(req: &CodeRequest) -> Result<usize> {
    Ok(build(req)?.iter().map(|m| estimate_tokens(&m.content) + PER_MESSAGE).sum())
}

/// The estimated tokens one file adds to the prompt.
pub fn file_tokens(f: &ContextFile) -> usize {
    estimate_tokens(&file_block(f))
}

/// `req` with files left out (last resort: in order, each one that no longer fits) so the
/// prompt fits `budget`. A rewrite's target always stays; if it can't fit, or nothing can, the
/// error says how big it is and how big the window is.
pub fn fit(req: &CodeRequest, budget: TokenBudget, model: &str) -> Result<Fitted> {
    let all = request_tokens(req)?;
    let limit = budget.prompt_tokens();
    if budget.scaled(all) <= limit {
        return Ok(Fitted { request: req.clone(), dropped: Vec::new(), tokens: budget.scaled(all), budget });
    }
    let costs: Vec<usize> = req.files.iter().map(file_tokens).collect();
    // The prompt without any file block: instructions, the ask (and a rewrite's selected lines).
    let frame = all.saturating_sub(costs.iter().sum::<usize>());
    let target = match &req.task {
        CodeTask::Rewrite(t) => Some(t.path.as_str()),
        CodeTask::Suggest => None,
    };
    let mut used = frame;
    if let Some(t) = target {
        let i = req.files.iter().position(|f| f.path == t).ok_or_else(|| bad(format!("{t} is not among the files sent")))?;
        used += costs[i];
        if budget.scaled(used) > limit {
            return Err(too_big(&format!("{t} alone is"), budget.scaled(used), budget, model));
        }
    }
    let mut files = Vec::new();
    let mut dropped = Vec::new();
    for (f, &cost) in req.files.iter().zip(&costs) {
        if Some(f.path.as_str()) == target {
            files.push(f.clone());
        } else if budget.scaled(used + cost) <= limit {
            used += cost;
            files.push(f.clone());
        } else {
            dropped.push((f.path.clone(), budget.scaled(cost)));
        }
    }
    if files.is_empty() {
        let smallest = costs.iter().min().copied().unwrap_or(0);
        return Err(too_big("even the smallest file is", budget.scaled(frame + smallest), budget, model));
    }
    let request = CodeRequest { files, ..req.clone() };
    Ok(Fitted { request, dropped, tokens: budget.scaled(used), budget })
}

fn too_big(what: &str, tokens: usize, b: TokenBudget, model: &str) -> CoreError {
    bad(format!(
        "{what} about {} tokens, but {model} is served with a {}-token context ({} for the prompt after room for the answer). Pick a smaller file or range, or a model with a bigger context.",
        thousands(tokens),
        b.context,
        thousands(b.prompt_tokens())
    ))
}

fn bad(msg: impl Into<String>) -> CoreError {
    CoreError::new(ErrorKind::BadRequest, msg)
}

/// A server's "the prompt is bigger than my context" refusal, read from its error text:
/// llama.cpp's `exceed_context_size_error` (`n_prompt_tokens`, `n_ctx`), vLLM's and OpenAI's
/// "maximum context length is N tokens … you requested M tokens" / `context_length_exceeded`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Overflow {
    /// What the server counted for the prompt.
    pub prompt_tokens: Option<u32>,
    /// The window the server has.
    pub context: Option<u32>,
}

impl Overflow {
    /// Reads `message`; `None` when it isn't a context-size refusal.
    pub fn parse(message: &str) -> Option<Self> {
        let m = message.to_ascii_lowercase();
        let is = m.contains("exceed_context_size")
            || m.contains("context_length_exceeded")
            || m.contains("maximum context length")
            || (m.contains("exceeds the available context size"));
        if !is {
            return None;
        }
        let prompt_tokens = json_number(&m, "n_prompt_tokens")
            .or_else(|| number_after(&m, "request ("))
            .or_else(|| number_after(&m, "you requested "))
            .or_else(|| number_after(&m, "resulted in "));
        let context = json_number(&m, "n_ctx")
            .or_else(|| number_after(&m, "available context size ("))
            .or_else(|| number_after(&m, "maximum context length is "));
        Some(Self { prompt_tokens, context })
    }

    /// What to tell the user instead of the server's JSON.
    pub fn message(&self, model: &str) -> String {
        let asked = self.prompt_tokens.map(|n| format!(" (about {} tokens)", thousands(n as usize))).unwrap_or_default();
        let ctx = self.context.map(|c| format!("its {}-token context", c)).unwrap_or_else(|| "its context".into());
        format!(
            "The request{asked} is too big for {model}: it is served with {ctx}. Take files out of the context (f), or pick a model with a bigger context."
        )
    }
}

fn json_number(m: &str, key: &str) -> Option<u32> {
    let i = m.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = m[i..].trim_start().strip_prefix(':')?;
    leading_number(rest.trim_start())
}

fn number_after(m: &str, prefix: &str) -> Option<u32> {
    let i = m.find(prefix)? + prefix.len();
    leading_number(&m[i..])
}

fn leading_number(s: &str) -> Option<u32> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit() || *c == ',').filter(|c| *c != ',').collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::Target;

    fn file(path: &str, bytes: usize) -> ContextFile {
        let line = "x = compute(a, b)  # pad\n";
        let text: String = line.repeat(bytes / line.len() + 1);
        ContextFile { path: path.into(), text }
    }

    fn suggest(files: Vec<ContextFile>) -> CodeRequest {
        CodeRequest { task: CodeTask::Suggest, instruction: "tidy".into(), files }
    }

    #[test]
    fn budget_keeps_room_for_the_answer_and_a_margin() {
        let b = TokenBudget::for_context(32_768);
        assert_eq!(b.output, 8192);
        assert_eq!(b.prompt_tokens(), 32_768 - 8192 - 32_768 / 20 - 64);
        let small = TokenBudget::for_context(4096);
        assert_eq!(small.output, 1024);
        assert!(small.prompt_tokens() > 2500 && small.prompt_tokens() < 3072);
        let tiny = TokenBudget::for_context(1024);
        assert!(tiny.output <= 512 && tiny.prompt_tokens() > 0);
        assert_eq!(b.with_ratio(0.5).ratio, 1.0);
        assert_eq!(b.with_ratio(1.3).scaled(100), 130);
    }

    #[test]
    fn the_estimate_is_on_the_high_side_for_code() {
        // 215 KiB of code was 91.7k tokens on llama.cpp; the estimate must not come in under it.
        let f = file("big.py", 215 * 1024);
        assert!(file_tokens(&f) > 91_723, "{}", file_tokens(&f));
    }

    #[test]
    fn a_request_that_fits_is_unchanged() {
        let req = suggest(vec![file("a.py", 2000), file("b.py", 2000)]);
        let f = fit(&req, TokenBudget::for_context(32_768), "m").unwrap();
        assert_eq!(f.request, req);
        assert!(f.dropped.is_empty() && f.note("m").is_empty());
        assert!(f.tokens <= f.budget.prompt_tokens());
    }

    #[test]
    fn the_reported_case_is_trimmed_to_a_32k_window() {
        // 22 files, 215 KiB, against a 32k window: files go until the prompt fits.
        let files: Vec<ContextFile> = (0..22).map(|i| file(&format!("ld4/m{i:02}.py"), 10 * 1024)).collect();
        let req = suggest(files);
        assert!(request_tokens(&req).unwrap() > 32_768);
        let b = TokenBudget::for_context(32_768);
        let f = fit(&req, b, "inkling-2-mini").unwrap();
        assert!(!f.dropped.is_empty());
        assert_eq!(f.request.files.len() + f.dropped.len(), 22);
        assert!(request_tokens(&f.request).unwrap() <= b.prompt_tokens());
        // The running estimate matches a fresh one, give or take rounding per file.
        assert!(f.tokens.abs_diff(request_tokens(&f.request).unwrap()) <= 22, "{}", f.tokens);
        // Kept files keep their order, and the first ones are the ones kept.
        assert_eq!(f.request.files[0].path, "ld4/m00.py");
        let note = f.note("inkling-2-mini");
        assert!(note.contains(&format!("Sent {} of 22 files", f.request.files.len())) && note.contains("32.8k context"), "{note}");
    }

    #[test]
    fn a_big_file_is_skipped_and_smaller_later_ones_still_go() {
        let req = suggest(vec![file("small1.py", 1000), file("huge.py", 200_000), file("small2.py", 1000)]);
        let f = fit(&req, TokenBudget::for_context(32_768), "m").unwrap();
        let kept: Vec<&str> = f.request.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(kept, ["small1.py", "small2.py"]);
        assert_eq!(f.dropped[0].0, "huge.py");
    }

    #[test]
    fn a_rewrite_target_is_always_kept_or_refused() {
        let task = CodeTask::Rewrite(Target { path: "z.py".into(), range: None });
        let req = CodeRequest { task: task.clone(), instruction: "faster".into(), files: vec![file("a.py", 30_000), file("z.py", 20_000)] };
        let f = fit(&req, TokenBudget::for_context(16_384), "m").unwrap();
        assert_eq!(f.request.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["z.py"]);
        let req = CodeRequest { task, instruction: "faster".into(), files: vec![file("z.py", 100_000)] };
        let e = fit(&req, TokenBudget::for_context(16_384), "m").unwrap_err();
        assert_eq!(e.kind, ErrorKind::BadRequest);
        assert!(e.message.contains("z.py alone is about") && e.message.contains("16384-token context"), "{}", e.message);
    }

    #[test]
    fn nothing_fitting_is_an_error_not_an_empty_request() {
        let req = suggest(vec![file("a.py", 100_000), file("b.py", 120_000)]);
        let e = fit(&req, TokenBudget::for_context(8192), "m").unwrap_err();
        assert!(e.message.contains("even the smallest file"), "{}", e.message);
    }

    #[test]
    fn a_server_ratio_makes_the_fit_stricter() {
        let files: Vec<ContextFile> = (0..10).map(|i| file(&format!("f{i}.py"), 8000)).collect();
        let req = suggest(files);
        let b = TokenBudget::for_context(32_768);
        let plain = fit(&req, b, "m").unwrap();
        let strict = fit(&req, b.with_ratio(1.5), "m").unwrap();
        assert!(strict.request.files.len() < plain.request.files.len());
    }

    #[test]
    fn reads_llama_cpp_vllm_and_openai_refusals() {
        let llama = r#"bad request 400: {"error":{"code":400,"message":"request (91723 tokens) exceeds the available context size (32768 tokens), try increasing it","type":"exceed_context_size_error","n_prompt_tokens":91723,"n_ctx":32768}}"#;
        assert_eq!(Overflow::parse(llama), Some(Overflow { prompt_tokens: Some(91_723), context: Some(32_768) }));
        let text_only = "request (91723 tokens) exceeds the available context size (32768 tokens), try increasing it";
        assert_eq!(Overflow::parse(text_only), Some(Overflow { prompt_tokens: Some(91_723), context: Some(32_768) }));
        let vllm = "This model's maximum context length is 32768 tokens. However, you requested 91723 tokens (83531 in the messages, 8192 in the completion).";
        assert_eq!(Overflow::parse(vllm), Some(Overflow { prompt_tokens: Some(91_723), context: Some(32_768) }));
        let openai = r#"{"error":{"code":"context_length_exceeded","message":"Your input exceeds the context window of this model."}}"#;
        assert_eq!(Overflow::parse(openai), Some(Overflow::default()));
        assert_eq!(Overflow::parse("bad request 400: unknown model"), None);
        let msg = Overflow::parse(llama).unwrap().message("inkling-2-mini");
        assert!(msg.contains("91.7k tokens") && msg.contains("32768-token context") && !msg.contains('{'), "{msg}");
    }

    #[test]
    fn thousands_reads_well() {
        assert_eq!(thousands(950), "950");
        assert_eq!(thousands(32_768), "32.8k");
        assert_eq!(thousands(91_000), "91k");
    }
}
