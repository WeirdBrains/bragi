# Handoff log

A running record of finished work on Bragi, newest first. Each entry says what changed, the commits, which checks ran and which did not, what is left and what comes next. Owner: [m3talsmith](https://github.com/m3talsmith). See also [ROADMAP.md](ROADMAP.md), [CHANGELOG.md](CHANGELOG.md) and [AGENTS.md](AGENTS.md).

## 2026-11-20: LF-21, delete a message, a conversation or all history (v0.1.1)

**What changed**
- Desktop and phone can delete a single message (hover or long-press; deleting a question asks whether its answer goes too), a whole conversation (history list) and all history (Settings, behind a typed confirm). Message and conversation deletes show an Undo for a few seconds; clear-all has no undo.
- Deletes go through `bragi-core` (new `history/forget.rs`), and the Flutter bridge was regenerated for the new calls. Deleting removes the data from disk: the messages, their thinking and attachments, older stored versions and the on-disk index and cache entries.
- Synced deletes travel as tombstones with the next sync, so a deleted item does not come back from a linked device. No `weirdnet-*` crate was changed.
- A deleted message and its thinking never go back into the model context.
- The bubble layout keeps room beside the text for reactions (LF-23).

**Commits**: [`4196f94`](https://github.com/WeirdBrains/bragi/commit/4196f94) (the feature), then the commit that adds this entry, the phone test updates and a build fix for the desktop bubble.

**Checks** (whole workspace): `cargo fmt --check`, build, `cargo clippy -D warnings`, `cargo test` (including `tests/delete.rs`), `flutter analyze`, and the Flutter scenario test in demo mode (message delete with Undo, conversation delete, clear all).
**Not tested**: a real phone; a delete synced between two real devices (covered by the core tests only).

**Left**: nothing known.
**Next**: LF-23, reactions.

## 2026-11-20: LF-19, the model's thinking (v0.1.1)

**What changed**
- A collapsible Thinking section sits above the reply on the desktop and phone apps. While the model reasons it is open and streams live with the elapsed time; once the answer starts it folds to a one-line "Thought for Ns" header (our choice: the answer stays the focus, one click or tap opens the reasoning again). Before anything arrives the reply says "Waiting for a reply…".
- Reasoning comes from the stream's `reasoning_content` field (what llama.cpp with `--reasoning-format` and vLLM send, and what the Ymir bridge passes through as reasoning frames), or from a `<think>…</think>` block at the start of the text (Qwen3 served without a reasoning parser), split in [`think.rs`](crates/bragi-core/src/think.rs).
- It is stored with the message (and synced with the conversation) but kept out of the answer body, out of Copy, and out of the context sent back to the model.
- The reasoning is drawn with the same Markdown renderer as answers (desktop and phone). The bubble layout leaves room beside the text for delete (LF-21) and reactions (LF-23).
- Bridge regenerated for the new `reasoning` event and message fields.

**Commits**: [`4e1e5ae`](https://github.com/WeirdBrains/bragi/commit/4e1e5ae) (the feature, rebased onto the Markdown work), then the commit that adds this entry and draws the reasoning as Markdown on the desktop.

**Checks** (whole workspace): `cargo fmt --check`, build, `cargo clippy -D warnings`, `cargo test` (including `tests/thinking.rs` for both formats), `flutter analyze`, the Flutter scenario test in demo mode, and the headless desktop scenario (virtual X server, clean environment, throwaway profile; screenshots 27 to 29 show reasoning, the folded header and the reopened section).
**Not tested**: a real phone; a real reasoning model through a live Ymir node (checked against recorded streams of two models and the demo model); very long reasoning on small screens.

**Left**: nothing known.
**Next**: LF-21, delete a message, a conversation or all history.

## 2026-11-20: LF-20, Markdown in replies (v0.1.1)

**What changed**
- Replies render as Markdown on the desktop app and the phone app: headings, lists, tables, block quotes, bold, italic, links, inline code and fenced code blocks.
- One parser, in `bragi-core` ([`markdown.rs`](crates/bragi-core/src/markdown.rs)), feeds both apps; the phone reads it through the bridge (`markdown_json`). Raw HTML stays plain text and images are shown as links, never fetched.
- Code blocks are highlighted ([`highlight.rs`](crates/bragi-core/src/highlight.rs)) and have a Copy button; each finished reply has a Copy button that gives its raw Markdown.
- Links open in the system browser (desktop; `url_launcher` on the phone, with the Android `queries` entries it needs).
- Streaming-safe: while a reply arrives, half-typed markers are not shown and an unclosed code fence is drawn as code to the end, so the layout does not jump.
- Replies stay selectable on the phone. The message list keeps the [LF-22](#2026-11-20-lf-22-chat-scrolling-with-a-fixed-prompt) scroll container, so the thinking section (LF-19), delete (LF-21) and reactions (LF-23) can sit beside each reply's text.

**Commits** on main: see the LF-20 commits in the [commit list](https://github.com/WeirdBrains/bragi/commits/main) (feature commit `3ad4a61`, later fixes follow it).

**Checks** (whole workspace): `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, build of `bragi-desktop` and `bragi-bridge`, `flutter analyze`, the Flutter scenario test in demo mode, and the headless desktop scenario (virtual X server, clean environment, throwaway profile; screenshots 25 and 26 show streaming and finished Markdown).
**Not tested**: a real phone; very long replies or very wide tables on small screens; languages other than Rust, Dart and a few common ones in the highlighter.

**Left**: no syntax highlighting for every language (unknown languages show as plain code); no images in replies.
**Next**: LF-19, show the model's thinking.

## 2026-11-20: LF-22, chat scrolling with a fixed prompt

**What changed**: the chat scrolls (the desktop message list is a single `#messages` scroll container; the phone uses a `ListView.builder`), the prompt field stays in a fixed position, the view sticks to the newest text while it streams, and a Jump to latest button appears once the reader scrolls up.

**Commits**: [`22eb2f2`](https://github.com/WeirdBrains/bragi/commit/22eb2f2), [`ebef36c`](https://github.com/WeirdBrains/bragi/commit/ebef36c) (main).

**Checks**: the `bragi-desktop` checks and the virtual-display desktop scenario passed; `flutter analyze` is clean.
**Not tested**: a real phone.

**Left**: nothing known.
**Next**: LF-20 (Markdown).

## 2026-11-20: LF-10, the Link a device notice and placeholder

**What changed**: the "Added the node's address; looking for it…" notice no longer sticks. It becomes "Connected to <node>" when the node answers and clears itself, or shows advice after a timeout; any other action clears it, on desktop and phone. The Link a device placeholder now fits its field.

**Commits**: [`8898606`](https://github.com/WeirdBrains/bragi/commit/8898606), [`8736629`](https://github.com/WeirdBrains/bragi/commit/8736629) (main).

**Checks**: fmt, clippy and tests passed.
**Not tested**: the headless GUI check was not run for this change.

**Left**: nothing known.
**Next**: LF-22 (chat scrolling).
