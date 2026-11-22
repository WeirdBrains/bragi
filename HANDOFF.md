# Handoff log

A running record of finished work on Bragi, newest first. Each entry says what changed, the commits, which checks ran and which did not, what is left and what comes next. Owner: [m3talsmith](https://github.com/m3talsmith). See also [ROADMAP.md](ROADMAP.md), [CHANGELOG.md](CHANGELOG.md) and [AGENTS.md](AGENTS.md).

## 2026-11-21: LF-30 M4, the Models screen (branch only)

**What changed**
- [`models.rs`](crates/bragi-core/src/models.rs) in `bragi-core`: `ModelsManager` runs Ymir's `/weirdnet-backends/models/1` protocol through `weirdnet-backends` 0.3's `ModelsClient`, with this device's certificate. Search asks the Hugging Face Hub from this device (anonymous, `weirdnet-model`'s `Hub`; a node searches for us if the Hub is unreachable from here) and then asks every ready node how each hit fits (`files`), one `Fit` event per repo and node. Pull goes to one node, `auto` (the node that holds the biggest model of the repo, with the reason in words) or all ready nodes, streams `Progress`, and cancels by dropping the stream (the node keeps the partial file; pulling again resumes). List and remove are per node; remove refuses without `confirmed`.
- [`models_core.rs`](crates/bragi-core/src/models_core.rs): `Core::models_overview`, `models_search`, `models_pull`, `models_remove`; a run is cancelled with the existing `Core::cancel`.
- The bridge ([`bragi.rs`](crates/bragi-bridge/src/api/bragi.rs), regenerated): `models_overview`, `models_search`, `models_pull`, `follow_models`, `models_remove`.
- Flutter: [`models_ui.dart`](app/lib/src/models_ui.dart), reached from the drawer and from Settings. Desktop (GPUI): [`models.rs`](crates/bragi-desktop/src/view/models.rs), a Models button in the sidebar. Both show nodes with their status, search with fit badges, a target picker (auto, a node, all), a quantization choice, progress bars with Cancel, per-node lists, and a remove confirm.
- Demo: the demo nodes now serve models/1 behind the same `RosterAuth` Ymir uses ([`demo.rs`](crates/bragi-core/src/demo.rs)), with a mock Hub ([`demo_models.rs`](crates/bragi-core/src/demo_models.rs)) whose repos have real sizes for the fit verdicts and one tiny repo that downloads slowly enough to see progress.
- Roadmap B7 in [`roadmap.toml`](roadmap.toml) (and the generated roadmap pages).

**Owner auth** (read from `weirdnet-backends` 0.3, `weirdnet-agent` 0.5 and Ymir main): `ymird` serves models/1 with plain `RosterAuth` (`Roster::check_caller`), without `require_roles` and without the `[model] callers` allow-list (that list only narrows raw infer/2 on a model node). So any device linked to the owner's roster is admitted, a Bragi device included, provided it presents its certificate (it does) and the node's roster has it (the roster syncs after linking). `RosterAuth` refuses with `unknown-device`, `revoked`, `wrong-user` or `needs-admission`; Bragi shows that as "Needs owner device" with the reason, and when every node refuses it turns the controls off behind a banner. Proven in [`models.rs`](crates/bragi-core/tests/models.rs): a linked device is admitted by the demo nodes' `RosterAuth` over the real Weirdnet stack and manages models; an unlinked one gets `NotPaired`. Nothing is needed from Ymir or weirdnet for this.

**Checks run** (whole workspace): `cargo fmt --check`, build, `cargo clippy -D warnings`, `cargo test` (the mock-node tests: fit per node, auto, one node, all, progress, cancel and resume, remove with and without confirm, a node that refuses, a node that is not a model node; the live-stack test above), `flutter analyze`, the Flutter tests in demo mode with `BRAGI_BRIDGE_LIB` ([`models_test.dart`](app/test/models_test.dart), including a device every node refuses), and the headless desktop scenario (virtual X server, clean environment, throwaway profile; screenshots 40 to 46). Read-only check against the real Hub: a live search through `weirdnet-model`'s `Hub`.

**Not tested**: a real Ymir node (no real pull or remove was run on any node); a real phone; iOS and Android builds with the new dependencies (`weirdnet-backends` with `node` pulls `weirdnet-model`); the Flutter screenshots use the test font (boxes for text), so the desktop screenshots are the readable ones; Windows and macOS CI for this branch (CI does not run on branch pushes).

**What's left**: Ymir's design doc should list the Bragi Models screen under the Bragi milestones; a check against a real node by the owner; quantization choice by file name; showing a node's free disk space.

**What's next**: the Bragi MCP server on `weirdnet-mcp` 0.6, then LF-12 (chat files).

## 2026-11-21: LF-18, Weirdnet 0.5 and image and audio attachments (branch only)

**What changed**
- `weirdnet-agent` 0.5.0 (the only Weirdnet crate Bragi's code calls that has a new release; `weirdnet-net`, `-sync` and `-device-link` stay as they are). A message's `content` is now text or parts, so the call sites that read it use `.text()`.
- Images (PNG, JPEG, GIF, WebP, up to 5 MiB) and audio (WAV, MP3, up to 10 MiB) can be attached to a message, next to text files. They go to the model as content parts (`image_url` with a data URL, `input_audio`), stay with the message and sync like a text attachment, and every later turn sends them again (the model has the whole conversation). A picture or sound can be sent with no words.
- The catalog shows which models read images and audio (`ModelInfo::vision` and `audio`, from `CatalogModel::inputs`). With no model picked, the first warm one that reads them is used; a picked model that can't is refused up front with the models that can named. Agent mode takes text and text files only (a task's input is a string) and says so.
- Desktop: the attach path box takes an image or audio file; chips and message lines say `(image)` or `(audio)`. Mobile: the same attach button opens the system picker, which already lets you pick any file; chips and bubbles show the kind.
- The bridge: `AttachmentView` gains `kind`, `mime` and `data` (history views leave `data` empty); `ModelView` gains `vision` and `audio`; `flutter_rust_bridge_codegen generate` was re-run.
- Demo nodes: `gemma-4-e4b` takes images and audio, the others text only; a model that was sent media says "I can see N image(s) and M sound(s)." so the tests can check what arrived.
- Kept minimal for LF-12 (chat files) to build on: media travels inline in the request and is stored in the message, so it stays small. Larger files need content roots.

**Checks run:** see the push; `cargo fmt --check`, build, `cargo clippy -D warnings` and `cargo test` on the whole workspace, `flutter analyze` and the Flutter tests in demo mode.

**Not tested:** a real phone; a real picture sent to a real vision model (the demo and mock nodes only); two real devices syncing a message with a picture; the system picker on Android and iOS (the demo build attaches a sample text file instead); a visual check of the desktop chips (no GUI run for this change). Ymir sends the parts on to llama.cpp in its LF-18 branch, which is separate.

**What's left:** thumbnails of attached pictures in the chat; a player for attached audio; recording audio; remote image URLs (nodes never fetch them); files larger than a message takes (LF-12).

**What's next:** LF-12 (chat files); LF-16, the Ymir MCP server, comes after this branch.

## 2026-11-21: LF-3, Bragi v0.1.1 cut

**What changed**
- Cut v0.1.1 from `c703af8` with the vendored release script (`scripts/weirdnet_release.py`): the version bump (0.1.0+2 to 0.1.1+3, so the Android versionCode is 3 per ABI offset 2003/1003/4003), the CHANGELOG section for everything since 0.1.0, the `release: v0.1.1` commit, the `v0.1.1` tag, and one normal push of `main` and the tag. The Release workflow builds every platform from the tag.
- v0.1.1 holds: node addresses in links and pasted by hand, the clearer no-node message, the Link a device notice fix, the scrolling chat with a fixed composer, Markdown replies, the collapsible Thinking section, delete from history with undo, and emoji reactions.

**Checks** (before the bump commit, whole workspace): `cargo fmt --check`, build, `cargo clippy -D warnings`, `cargo test`, `flutter analyze` and the Flutter tests in demo mode.
**Not tested**: a real phone, two real devices syncing, a live reasoning model, signed builds (no signing keys exist; the minisign job only prints a notice, the Android APKs use the debug key).

**Published**: the [v0.1.1 release](https://github.com/WeirdBrains/bragi/releases/tag/v0.1.1) has 31 files and a `SHA256SUMS`, built by the Release workflow (all jobs passed) and scanned for private details first. TAUR `bragi-bin` is 0.1.1-1, `packaging/nix/release.json` and the site's `latestVersion` are 0.1.1. The Android apks are signed with the debug key (no release key exists) and the minisign job only printed its notice (no key).

**Next**: LF-18, then the v1.0.0 work on a branch.

## 2026-11-21: LF-23, emoji reactions (v0.1.1)

**What changed**
- Any message, the model's replies too, takes emoji reactions on the desktop and phone apps. The quick bar is 👍 👎 ❤️ 😂 🎉 🤔 plus a picker for more (hover on desktop with a "+" for the picker; long-press on the phone). Reactions show as small pills under the message; click or tap a pill, or pick the same emoji again, to remove it. Up to 12 per message; only emoji are accepted.
- They live in `bragi-core` ([`reactions.rs`](crates/bragi-core/src/reactions.rs), `Core::react`), saved on the message in the conversation record, so they sync with the conversation, survive restarts and are removed with their message by the LF-21 delete (an Undo brings them back). Older records without the field still load.
- Per the owner's request they go to the model: the next turn carries a short note after the reacted message, such as `[The user reacted to this reply with 👎]` (or `…to their own message with 🤔`). Remove the reaction and the note is gone. Reactions are not part of Copy.
- `Core::reaction_events()` is a broadcast of every add and remove (with the model, node, and a good/bad flag for 👍/👎), as the hook for forwarding feedback. Nothing consumes it yet; see Left.
- Bridge regenerated (`react`, `quick_reactions`, `MessageView.reactions`). No `weirdnet-*` crate was changed.

**Commits**: [`1023daa`](https://github.com/WeirdBrains/bragi/commit/1023daa) (the feature, on `wip/lf23-reactions`), then the commit that adds this entry, the phone analyzer fixes and the test.

**Checks** (whole workspace): `cargo fmt --check`, build, `cargo clippy -D warnings`, `cargo test` (including `tests/reactions.rs`: toggle, persistence, events, delete and undo), `flutter analyze`, the Flutter scenario test in demo mode (reaction bar, pills, removal), and the headless desktop scenario (virtual X server, clean environment, throwaway profile; screenshots 19 and 20 show the pills and the picker).
**Not tested**: a real phone; reactions synced between two real devices (the reaction rides on the message record the sync tests already cover); a live model reading the note.

**Left**: feedback to agents or Ymir. Read-only check of the Ymir bridge and the `weirdnet-agent` 0.5 protocol (`infer/1`, `infer/2`, `task`, `catalog` messages): there is no feedback or rating message on any of them, so reactions only reach the model as context. Sending 👍/👎 as feedback needs a new message in those crates, a decision for the Weirdnet side; `Core::reaction_events()` is ready to feed it.
**Next**: the v0.1.1 release cut, which waits for the owner.

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

## 2026-11-21: Weirdnet 0.5 image and audio work merged to main

**What changed**

- `feature/weirdnet-0.5-content-parts` (head `389844e`) is merged into main with a merge commit: Weirdnet 0.5 agent, content parts, and image and audio attachments in the composer (core, bridge, desktop, phone). The branch is kept.

**Checks run:** `cargo fmt --check`, `cargo build --workspace`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace`, `flutter analyze` and `flutter test --dart-define=BRAGI_DEMO=true` (with the bridge library) on the merge result, all passing.

**Not tested:** real GPU vision, phone. The Platforms workflow has not run on this code.

**What's left:** other branches should merge main.

**What's next:** the OpenAI endpoints work.

## 2026-11-21: LF-11, OpenAI-compatible endpoints, Settings, sessions and folders

**What changed**: `bragi-core` gains OpenAI-compatible endpoints (any URL, optional API key; test, reorder, fallback-only or alongside the Weirdnet pool), Weirdnet links managed from Settings, per-session targets (the pool, or an endpoint and model), and nested session folders. Deleting an endpoint asks first and rebinds or unbinds its chats; history is never deleted. Both shells have it: the Flutter app (desktop and phone) and the GPUI desktop shell (Settings sections for endpoints and links, the form and delete cards, the folder tree in the sidebar, the "Sends to" target picker, `view/endpoints.rs`). Roadmap: new B6 line (site and [ROADMAP.md](ROADMAP.md) regenerated).

**API keys**: stored in the OS keychain when a shell opts in (`Core::use_os_keychain`): Secret Service (libsecret) on Linux, Keychain on macOS, Credential Manager on Windows, through the `keyring` crate. Keys already in the sealed file move over on first use. When there is no keychain (no Secret Service running; iOS and Android, which have no store wired in yet) keys stay in the sealed file store (`secrets.enc`, ChaCha20-Poly1305). Keys are never logged, never in `endpoints.json` and never synced. The demo and tests never touch a real keychain.

**For main**: this branch pins `weirdnet-device-link =0.3.2` and `weirdnet-net =0.8.2` (the Weirdnet Lead yanked 0.3.1 and 0.8.1; content-only patches). Main still pins the yanked versions, so the same two-line `Cargo.toml` bump (and `cargo update -p weirdnet-device-link -p weirdnet-net`) is needed on main if this branch does not merge first.

**Branch**: `feature/openai-endpoints-v2` (main merged in; not merged to main).

**Checks**: fmt, clippy -D warnings and `cargo test --workspace` (mock OpenAI server: streaming, errors, auth, models list; keychain move, clear, delete and fallback with an in-memory store) passed; the real Secret Service round trip passed in a private session bus with a throwaway gnome-keyring (`cargo test -p bragi-core -- --ignored os_keychain`); a live OpenAI-compatible server (the Ymir loopback bridge) passed list, test and a streamed chat (`cargo test -p bragi-core --test live -- --ignored`, with `BRAGI_LIVE_URL` and `BRAGI_LIVE_KEY`); `flutter analyze` clean; `flutter test --dart-define=BRAGI_DEMO=true` passed; the GPUI desktop scenario (`scripts/desktop-scenario.sh`, Xvfb) passed with the new endpoints, links, folders and rebind steps.
**Not tested**: macOS Keychain and Windows Credential Manager (compiled for Linux only); a real phone; iOS and Android keychains (not implemented); the Flutter Linux window screenshots were taken from a temporary Linux platform scaffold and a demo tour that are not part of the repo.

**Left**: iOS Keychain and Android Keystore (keyring-core store crates need device testing; those platforms keep the sealed file).
**Next**: LF-12.

## 2026-11-21: LF-11 endpoints, Settings and sessions merged to main

**What changed**

- Merged `feature/openai-endpoints-v2` into main with a merge commit. Main now has the `weirdnet-device-link =0.3.2` and `weirdnet-net =0.8.2` pins that replace the yanked versions.

**Checks run:** `cargo fmt --check`, build, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace`, `flutter analyze` and `flutter test --dart-define=BRAGI_DEMO=true` on the merged tree.

**Not tested:** macOS Keychain and Windows Credential Manager; the Platforms workflow (manual dispatch only) has not run on this merge.

**What's left:** iOS Keychain and Android Keystore.

**What's next:** LF-12.
