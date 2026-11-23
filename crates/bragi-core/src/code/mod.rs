//! Code suggestions and rewrites, shared by every shell that edits code (the VS Code / Cursor
//! extension through `bragi serve`, and the terminal app `bragi-tui`) so they never diverge.
//! Answers go through [`Core::complete`](crate::Core::complete), kept out of chat history.
//!
//! Two kinds of request share the answer reading and the diffs:
//!
//! - From an editor ([`EditorContext`], in `editor`): an inline suggestion at the cursor
//!   ([`suggest_prompt`]) or a rewrite of the selection or file ([`rewrite_prompt`]).
//! - On files and folders ([`prompt`]): suggestions across them, or a rewrite of a file or a
//!   line range, answered as `bragi-edit` blocks that can touch several files.
//!
//! The file flow:
//!
//! 1. [`context::gather`] reads the files and directories the user picked (respecting
//!    `.gitignore`, skipping binaries, secrets and oversize files).
//! 2. [`budget::fit`] leaves out the files that don't fit the model's context window (in tokens,
//!    with room kept for the answer), and [`budget::Overflow`] reads a server's refusal when the
//!    window was smaller than the catalog said.
//! 3. [`prompt::build`] turns a [`CodeRequest`] into prompt messages for
//!    [`Core::complete`](crate::Core::complete).
//! 4. [`prompt::parse_edits`] reads the answer into [`ProposedEdit`]s, and
//!    [`prompt::proposals`] turns those into one [`FileDiff`] per file.
//! 5. The shell shows each diff's hunks; the user accepts or rejects each one, and
//!    [`FileDiff::apply`] builds the file with only the accepted hunks.
//! 6. [`Journal::apply`] writes the files, keeping a copy of what was there, and
//!    [`Journal::undo_last`] puts it back.

pub mod budget;
pub mod context;
pub mod diff;
pub mod edit;
mod editor;
pub mod prompt;

pub use budget::{Fitted, Overflow, TokenBudget, fit};
pub use context::{ContextFile, Gathered, Limits, gather};
pub use diff::{FileDiff, Hunk, HunkLine, LineTag, SideBySideRow};
pub use edit::{Applied, FileChange, Journal, JournalEntry};
pub use editor::*;
pub use prompt::{CodeRequest, CodeTask, LineRange, ProposedEdit, Span, Target, parse_target};
