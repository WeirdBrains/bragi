# Handoff log

A running record of finished work on Bragi, newest first. Each entry says what changed, the commits, which checks ran and which did not, what is left and what comes next. Owner: [m3talsmith](https://github.com/m3talsmith). See also [ROADMAP.md](ROADMAP.md), [CHANGELOG.md](CHANGELOG.md) and [AGENTS.md](AGENTS.md).

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
