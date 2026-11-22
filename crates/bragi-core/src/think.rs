//! Reasoning written inline as `<think>…</think>` (Qwen3, DeepSeek-R1 and similar models served
//! without a reasoning parser), split out of the answer text client-side.
//!
//! Only a block that opens the answer counts (leading whitespace allowed): a model that writes
//! about `<think>` tags later in its answer keeps them. A tag split between stream pieces is
//! held back until it is whole, so none of it flashes in the answer.

/// A piece of the stream once split.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Reasoning(String),
    Text(String),
}

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Nothing decided yet: the answer may still open with `<think>`.
    Start,
    /// Inside the block.
    Inside,
    /// Just after `</think>`: the blank lines before the answer are dropped.
    Closed,
    /// Plain answer text from here on.
    After,
}

/// Streaming splitter. Feed it text pieces with [`push`](Self::push), then call
/// [`finish`](Self::finish) once the stream ends.
#[derive(Debug, Clone)]
pub struct ThinkSplit {
    state: State,
    held: String,
}

impl Default for ThinkSplit {
    fn default() -> Self {
        Self::new()
    }
}

impl ThinkSplit {
    pub fn new() -> Self {
        Self { state: State::Start, held: String::new() }
    }

    /// A splitter for text that continues an answer already shown: no block can open it.
    pub fn continuing() -> Self {
        Self { state: State::After, held: String::new() }
    }

    pub fn push(&mut self, piece: &str) -> Vec<Piece> {
        let mut out = Vec::new();
        self.held.push_str(piece);
        loop {
            match self.state {
                State::Start => {
                    let trimmed = self.held.trim_start();
                    if let Some(rest) = trimmed.strip_prefix(OPEN) {
                        self.held = rest.to_string();
                        self.state = State::Inside;
                        continue;
                    }
                    if OPEN.starts_with(trimmed) {
                        // Only whitespace or part of the tag so far.
                        return out;
                    }
                    self.state = State::After;
                }
                State::Inside => {
                    if let Some(at) = self.held.find(CLOSE) {
                        let reasoning = self.held[..at].to_string();
                        self.held = self.held[at + CLOSE.len()..].to_string();
                        push(&mut out, Piece::Reasoning(reasoning));
                        self.state = State::Closed;
                        continue;
                    }
                    let keep = partial_tail(&self.held, CLOSE);
                    let emit: String = self.held.drain(..self.held.len() - keep).collect();
                    push(&mut out, Piece::Reasoning(emit));
                    return out;
                }
                State::Closed => {
                    let trimmed = self.held.trim_start();
                    if trimmed.is_empty() {
                        self.held.clear();
                        return out;
                    }
                    self.held = trimmed.to_string();
                    self.state = State::After;
                }
                State::After => {
                    let t = std::mem::take(&mut self.held);
                    push(&mut out, Piece::Text(t));
                    return out;
                }
            }
        }
    }

    /// What was held back when the stream ends: an unclosed block is all reasoning.
    pub fn finish(&mut self) -> Vec<Piece> {
        let t = std::mem::take(&mut self.held);
        let mut out = Vec::new();
        match self.state {
            State::Inside => push(&mut out, Piece::Reasoning(t)),
            State::Start | State::After => push(&mut out, Piece::Text(t)),
            State::Closed => {}
        }
        self.state = State::After;
        out
    }
}

fn push(out: &mut Vec<Piece>, p: Piece) {
    match &p {
        Piece::Reasoning(t) | Piece::Text(t) if t.is_empty() => {}
        _ => out.push(p),
    }
}

/// The length of the longest end of `s` that could be the start of `tag`.
fn partial_tail(s: &str, tag: &str) -> usize {
    (1..tag.len().min(s.len() + 1)).rev().find(|&n| s.is_char_boundary(s.len() - n) && tag.starts_with(&s[s.len() - n..])).unwrap_or(0)
}

/// Splits a whole text into `(reasoning, answer)`. For messages saved with the tags in them.
pub fn split(text: &str) -> (String, String) {
    let mut s = ThinkSplit::new();
    let mut pieces = s.push(text);
    pieces.extend(s.finish());
    let (mut reasoning, mut answer) = (String::new(), String::new());
    for p in pieces {
        match p {
            Piece::Reasoning(t) => reasoning.push_str(&t),
            Piece::Text(t) => answer.push_str(&t),
        }
    }
    (reasoning, answer)
}

/// The Thinking section's title, the same in every shell: `Thinking… 4s` while the model reasons
/// (`done` false), `Thought for 4s` after.
pub fn label(elapsed_ms: u64, done: bool) -> String {
    let secs = elapsed_ms / 1000;
    let time = if secs >= 60 { format!("{}m {:02}s", secs / 60, secs % 60) } else { format!("{secs}s") };
    match (done, secs) {
        (false, 0) => "Thinking…".into(),
        (false, _) => format!("Thinking… {time}"),
        (true, 0) => "Thought for a moment".into(),
        (true, _) => format!("Thought for {time}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(pieces: &[&str]) -> (String, String) {
        let mut s = ThinkSplit::new();
        let mut all: Vec<Piece> = pieces.iter().flat_map(|p| s.push(p)).collect();
        all.extend(s.finish());
        let (mut r, mut t) = (String::new(), String::new());
        for p in all {
            match p {
                Piece::Reasoning(x) => r.push_str(&x),
                Piece::Text(x) => t.push_str(&x),
            }
        }
        (r, t)
    }

    #[test]
    fn a_leading_block_is_reasoning() {
        assert_eq!(run(&["<think>hm, 17*23</think>\n\n391"]), ("hm, 17*23".into(), "391".into()));
    }

    #[test]
    fn tags_split_between_pieces() {
        let pieces = ["\n<", "thi", "nk>\nlet me ", "see</", "thi", "nk", ">", "\n\n", "The ", "answer"];
        assert_eq!(run(&pieces), ("\nlet me see".into(), "The answer".into()));
    }

    #[test]
    fn every_split_point_gives_the_same_result() {
        let whole = "<think>é ü 中</think>\n\nanswer with </think> and <think> inside";
        let want = split(whole);
        assert_eq!(want, ("é ü 中".into(), "answer with </think> and <think> inside".into()));
        for (i, _) in whole.char_indices() {
            assert_eq!(run(&[&whole[..i], &whole[i..]]), want, "split at {i}");
        }
    }

    #[test]
    fn text_without_a_block_passes_through() {
        assert_eq!(run(&["Hello ", "<think> is a tag"]), (String::new(), "Hello <think> is a tag".into()));
        assert_eq!(run(&["<thin", "g>"]), (String::new(), "<thing>".into()));
        assert_eq!(run(&["  "]), (String::new(), "  ".into()));
    }

    #[test]
    fn an_unclosed_block_is_reasoning() {
        assert_eq!(run(&["<think>still going", " </thi"]), ("still going </thi".into(), String::new()));
    }

    #[test]
    fn reasoning_streams_before_the_close() {
        let mut s = ThinkSplit::new();
        assert_eq!(s.push("<think>one "), vec![Piece::Reasoning("one ".into())]);
        assert_eq!(s.push("two</"), vec![Piece::Reasoning("two".into())]);
        assert_eq!(s.push("think>Hi"), vec![Piece::Text("Hi".into())]);
    }

    #[test]
    fn a_continuation_never_opens_a_block() {
        let mut s = ThinkSplit::continuing();
        assert_eq!(s.push("<think>x</think>"), vec![Piece::Text("<think>x</think>".into())]);
    }

    #[test]
    fn labels() {
        assert_eq!(label(300, false), "Thinking…");
        assert_eq!(label(4200, false), "Thinking… 4s");
        assert_eq!(label(400, true), "Thought for a moment");
        assert_eq!(label(65_000, true), "Thought for 1m 05s");
    }
}
