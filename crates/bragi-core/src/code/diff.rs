//! Line diffs a person reviews hunk by hunk: compute, show (unified or side by side), and apply
//! only the hunks they accepted. Line endings are kept byte for byte, so applying every hunk
//! gives exactly the new text and applying none gives exactly the old.

use serde::{Deserialize, Serialize};
use similar::{Algorithm, DiffTag, capture_diff_slices, group_diff_ops};

/// Lines of context around each change (what `diff -u` uses).
pub const CONTEXT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineTag {
    Context,
    Removed,
    Added,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HunkLine {
    pub tag: LineTag,
    /// The line without its line ending.
    pub text: String,
    /// The line ended with a newline (only the file's last line may not).
    pub newline: bool,
    /// 1-based line numbers in the old and new file.
    pub old_no: Option<usize>,
    pub new_no: Option<usize>,
}

/// One reviewable change with its context. Starts are 0-based line indexes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    pub old_start: usize,
    pub old_len: usize,
    pub new_start: usize,
    pub new_len: usize,
    pub lines: Vec<HunkLine>,
}

/// One row of a side-by-side view: `(line number, text)` on each side.
pub type SideBySideRow = (Option<(usize, String)>, Option<(usize, String)>);

impl Hunk {
    /// `@@ -a,b +c,d @@` as `diff -u` writes it.
    pub fn header(&self) -> String {
        let start = |s: usize, len: usize| if len == 0 { s } else { s + 1 };
        format!("@@ -{},{} +{},{} @@", start(self.old_start, self.old_len), self.old_len, start(self.new_start, self.new_len), self.new_len)
    }

    pub fn added(&self) -> usize {
        self.lines.iter().filter(|l| l.tag == LineTag::Added).count()
    }

    pub fn removed(&self) -> usize {
        self.lines.iter().filter(|l| l.tag == LineTag::Removed).count()
    }

    /// Rows for a two-column view: context on both sides, a run of removed lines paired with the
    /// run of added lines that follows it.
    pub fn side_by_side(&self) -> Vec<SideBySideRow> {
        let mut rows = Vec::new();
        let mut i = 0;
        while i < self.lines.len() {
            let l = &self.lines[i];
            if l.tag == LineTag::Context {
                rows.push((l.old_no.map(|n| (n, l.text.clone())), l.new_no.map(|n| (n, l.text.clone()))));
                i += 1;
                continue;
            }
            let mut removed = Vec::new();
            while i < self.lines.len() && self.lines[i].tag == LineTag::Removed {
                removed.push(&self.lines[i]);
                i += 1;
            }
            let mut added = Vec::new();
            while i < self.lines.len() && self.lines[i].tag == LineTag::Added {
                added.push(&self.lines[i]);
                i += 1;
            }
            for k in 0..removed.len().max(added.len()) {
                let left = removed.get(k).and_then(|l| l.old_no.map(|n| (n, l.text.clone())));
                let right = added.get(k).and_then(|l| l.new_no.map(|n| (n, l.text.clone())));
                rows.push((left, right));
            }
        }
        rows
    }
}

/// The changes to one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiff {
    /// As shown (relative, `/` separators).
    pub path: String,
    pub old: String,
    pub new: String,
    /// The file doesn't exist yet.
    #[serde(default)]
    pub created: bool,
    pub hunks: Vec<Hunk>,
}

/// `text` split into lines, each keeping its line ending.
pub fn lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

fn diff_line(tag: LineTag, raw: &str, old_no: Option<usize>, new_no: Option<usize>) -> HunkLine {
    let newline = raw.ends_with('\n');
    let text = raw.strip_suffix('\n').unwrap_or(raw);
    let text = text.strip_suffix('\r').unwrap_or(text).to_string();
    HunkLine { tag, text, newline, old_no, new_no }
}

impl FileDiff {
    pub fn compute(path: impl Into<String>, old: impl Into<String>, new: impl Into<String>) -> Self {
        Self::compute_with(path, old, new, CONTEXT)
    }

    pub fn compute_with(path: impl Into<String>, old: impl Into<String>, new: impl Into<String>, context: usize) -> Self {
        let (old, new) = (old.into(), new.into());
        let (a, b) = (lines(&old), lines(&new));
        let ops = capture_diff_slices(Algorithm::Myers, &a, &b);
        let mut hunks = Vec::new();
        for group in group_diff_ops(ops, context) {
            let (Some(first), Some(last)) = (group.first(), group.last()) else { continue };
            let (_, o0, n0) = first.as_tag_tuple();
            let (_, o1, n1) = last.as_tag_tuple();
            let mut h = Hunk { old_start: o0.start, old_len: o1.end - o0.start, new_start: n0.start, new_len: n1.end - n0.start, lines: Vec::new() };
            for op in &group {
                let (tag, o, n) = op.as_tag_tuple();
                match tag {
                    DiffTag::Equal => {
                        for (i, j) in o.clone().zip(n.clone()) {
                            h.lines.push(diff_line(LineTag::Context, a[i], Some(i + 1), Some(j + 1)));
                        }
                    }
                    DiffTag::Delete => {
                        for i in o.clone() {
                            h.lines.push(diff_line(LineTag::Removed, a[i], Some(i + 1), None));
                        }
                    }
                    DiffTag::Insert => {
                        for j in n.clone() {
                            h.lines.push(diff_line(LineTag::Added, b[j], None, Some(j + 1)));
                        }
                    }
                    DiffTag::Replace => {
                        for i in o.clone() {
                            h.lines.push(diff_line(LineTag::Removed, a[i], Some(i + 1), None));
                        }
                        for j in n.clone() {
                            h.lines.push(diff_line(LineTag::Added, b[j], None, Some(j + 1)));
                        }
                    }
                }
            }
            hunks.push(h);
        }
        Self { path: path.into(), old, new, created: false, hunks }
    }

    /// No change at all.
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }

    /// The file with only the hunks marked `true` in `accepted` (missing entries count as
    /// rejected).
    pub fn apply(&self, accepted: &[bool]) -> String {
        let (old, new) = (lines(&self.old), lines(&self.new));
        let mut out = String::with_capacity(self.new.len().max(self.old.len()));
        let mut at = 0;
        for (i, h) in self.hunks.iter().enumerate() {
            for l in &old[at..h.old_start] {
                out.push_str(l);
            }
            let take = if accepted.get(i).copied().unwrap_or(false) {
                &new[h.new_start..h.new_start + h.new_len]
            } else {
                &old[h.old_start..h.old_start + h.old_len]
            };
            for l in take {
                out.push_str(l);
            }
            at = h.old_start + h.old_len;
        }
        for l in &old[at.min(old.len())..] {
            out.push_str(l);
        }
        out
    }

    /// The whole diff in `diff -u` form (`--- a/path`, `+++ b/path`, hunks).
    pub fn unified(&self) -> String {
        if self.hunks.is_empty() {
            return String::new();
        }
        let from = if self.created { "/dev/null".to_string() } else { format!("a/{}", self.path) };
        let mut s = format!("--- {from}\n+++ b/{}\n", self.path);
        for h in &self.hunks {
            s.push_str(&h.header());
            s.push('\n');
            for l in &h.lines {
                s.push(match l.tag {
                    LineTag::Context => ' ',
                    LineTag::Removed => '-',
                    LineTag::Added => '+',
                });
                s.push_str(&l.text);
                s.push('\n');
                if !l.newline {
                    s.push_str("\\ No newline at end of file\n");
                }
            }
        }
        s
    }

    /// `+added -removed` over every hunk.
    pub fn stats(&self) -> (usize, usize) {
        self.hunks.iter().fold((0, 0), |(a, r), h| (a + h.added(), r + h.removed()))
    }
}

/// Several files' diffs as one patch.
pub fn unified_all(diffs: &[FileDiff]) -> String {
    diffs.iter().map(FileDiff::unified).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\n";

    fn changed() -> String {
        OLD.replace("b\n", "B\n").replace("m\n", "m\nM2\n")
    }

    #[test]
    fn all_none_and_some_hunks() {
        let d = FileDiff::compute("x.txt", OLD, changed());
        assert_eq!(d.hunks.len(), 2, "two separate changes: {d:#?}");
        assert_eq!(d.apply(&[true, true]), changed());
        assert_eq!(d.apply(&[false, false]), OLD);
        assert_eq!(d.apply(&[]), OLD);
        assert_eq!(d.apply(&[true, false]), OLD.replace("b\n", "B\n"));
        assert_eq!(d.apply(&[false, true]), OLD.replace("m\n", "m\nM2\n"));
        assert_eq!(d.stats(), (2, 1));
    }

    #[test]
    fn unified_matches_diff_u() {
        let d = FileDiff::compute("x.txt", "one\ntwo\nthree\n", "one\n2\nthree\n");
        assert_eq!(d.unified(), "--- a/x.txt\n+++ b/x.txt\n@@ -1,3 +1,3 @@\n one\n-two\n+2\n three\n");
    }

    #[test]
    fn missing_final_newline_round_trips() {
        let (old, new) = ("x\ny", "x\ny\nz");
        let d = FileDiff::compute("f", old, new);
        assert_eq!(d.apply(&[true]), new);
        assert_eq!(d.apply(&[false]), old);
        assert!(d.unified().contains("\\ No newline at end of file"));
    }

    #[test]
    fn crlf_is_kept() {
        let (old, new) = ("a\r\nb\r\nc\r\n", "a\r\nB\r\nc\r\n");
        let d = FileDiff::compute("f", old, new);
        assert_eq!(d.apply(&[true]), new);
        assert_eq!(d.hunks[0].lines[1].text, "b");
    }

    #[test]
    fn side_by_side_pairs_runs() {
        let d = FileDiff::compute("f", "a\nb\nc\n", "a\nB\nB2\nc\n");
        let rows = d.hunks[0].side_by_side();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1], (Some((2, "b".into())), Some((2, "B".into()))));
        assert_eq!(rows[2], (None, Some((3, "B2".into()))));
    }

    #[test]
    fn identical_is_empty() {
        let d = FileDiff::compute("f", OLD, OLD);
        assert!(d.is_empty());
        assert_eq!(d.unified(), "");
        assert_eq!(d.apply(&[]), OLD);
    }
}
