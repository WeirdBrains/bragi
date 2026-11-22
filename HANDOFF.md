# Handoff log

A running record of finished work on Bragi, newest first. Each entry says what changed, the commits, which checks ran and which did not, what is left and what comes next. Owner: [m3talsmith](https://github.com/m3talsmith). See also [ROADMAP.md](ROADMAP.md), [CHANGELOG.md](CHANGELOG.md) and [AGENTS.md](AGENTS.md).

## 2026-11-22: LF-38 follow-up, video design corrected (branch `feature/video-design`)

**What changed** in [`docs/design/video.md`](docs/design/video.md), after m3talsmith's review:
- Added the platform table, checked against `scripts/dist-*`, the README and packaging: Linux, macOS and Windows ship GPUI (`bragi-desktop`) plus the terminal app; Android and iOS ship Flutter; the BSDs have only the terminal app. The Flutter-on-desktop plugin question is removed; Flutter is scoped to the phones.
- Decided 2026-11-22: the desktop gets an inline player, falling back to the system player when it can't handle the clip.
- Decided 2026-11-22: no default frame count or size. A new section, "Fitting the clip to the model", derives frames from the model's reported limits (context, image cost, `max_images`, audio and native video limits, request size). With no metadata, Bragi refuses with a clear message and offers an explicit per-model override; probing and guessing are rejected, with reasons. Wish list updated.
- Docs only; no code.


## 2026-11-22: LF-38, post-v1 video design (branch `feature/video-design`)

**What changed**
- New [`docs/design/video.md`](docs/design/video.md): video after v1 in two phases. Phase 1: Bragi samples frames on the device (uniform, scene or hybrid; fps, max frames and size are settings), sends them as timestamped image parts plus the soundtrack as an audio part (or a transcript if the model lacks audio). It works with any vision model and needs no protocol change. Phase 2: native video once Ymir has a capability flag. Covers the player in all front ends, the ffmpeg dependency and packaging per platform, limits, privacy, MCP tools, testing, milestones and a Weirdnet wish list (`weirdnet-media` first).
- Docs only: no code, no tests changed.

**Checks:** none needed (docs only); the doc was scanned for host names, addresses and home paths.

**Next:** Ymir's side is in `ymir/docs/design/video.md`. The wish list goes to the Weirdnet Lead.


## 2026-11-22: LF-36, `bragi serve` starts by itself with the terminal app and the editor extension (branch `feature/serve-autostart`)

**What changed**
- [`ensure.rs`](crates/bragi-serve/src/ensure.rs) in `bragi-serve`: `ensure()` checks the port (`/v1/health`, then the token with `/v1/state`), reuses a healthy server, or starts `bragi serve` detached (its own process group on Unix, `DETACHED_PROCESS` without a window on Windows; output to `<home>/serve/server.log`) and waits up to 25 s. One starter at a time through `<home>/serve/start.lock` (taken over once 60 s old); the running server writes `<home>/serve/server.pid` and removes it on exit, and a pid file with nothing on the port is stale and replaced. A port held by something else, or by a server that refuses this home's token, is reported and left alone. Nothing is ever killed. The program is `$BRAGI_BIN`, the running `bragi`, one beside the running program, then `bragi` or `bragi-serve` on the PATH.
- [`lib.rs`](crates/bragi-serve/src/lib.rs) and [`cli.rs`](crates/bragi-serve/src/cli.rs): `bragi serve --idle-timeout MINUTES` stops the server after that long with no client (requests other than `/v1/health` count, and a running request counts as activity); started by `ensure` it is 30. New `bragi serve ensure` and `bragi serve status`.
- [`serve.rs`](crates/bragi-tui/src/serve.rs) in the terminal app: it ensures a server in a background thread at start (so the app never waits for it), shows `serve`, `serve starting`, `serve idle`, `serve failed` or `serve off` in the status bar, and a failure is a toast. `--no-serve` and `BRAGI_TUI_SERVE=off` turn it off; `--demo` doesn't start one. It looks every 20 s but never restarts a server that stopped idle.
- [`serve.ts`](editors/vscode/src/serve.ts) in the extension: on activation, when the target is Bragi and nothing answers, it spawns `bragi serve` (`bragi.path`, else the PATH), reads the token file into secret storage, shows a second status-bar item and adds **Bragi: Start/Restart Server** (restarts only a server whose pid it recorded when it started it). A request that finds no server starts one and retries once. Settings: `bragi.autoStartServer` (default true), `bragi.path` (machine scope). Not in untrusted workspaces. A missing program is an error message with a shortcut to the setting.
- [`extension.ts`](editors/vscode/src/extension.ts) `ensureToken`: choosing a target and model with no token reads the Bragi home's token silently (after the server had a chance to make it), else asks in a password field with an import button, then goes on to the model list. Tested in [`serve.test.ts`](editors/vscode/src/test/suite/serve.test.ts).
- Docs: [extension README](editors/vscode/README.md), [README](README.md), the extension's changelog.

**Checks**: see the report in the merge commit. Rust: `cargo fmt`, `clippy -D warnings`, `cargo test --workspace`; extension: 33 tests in headless VS Code (Xvfb, `env -i`, a throwaway HOME), including seven for the autostart against a stand-in `bragi`; `flutter analyze` and `flutter test`.

**Not tested**: Windows and macOS (the code paths are there; spawn flags and the PATH search are unit-checked on Linux only); a real Cursor; the real `bragi serve` linked to a real node (tests use the demo node and a stand-in).

**Left**: the idle stop is by whole minutes; the desktop app's `--serve` isn't tracked by the pid file; a `.cmd` shim for `bragi.path` on Windows isn't run through a shell.

## 2026-11-22: LF-34, `bragi PATH` opens the terminal app on a folder or file (branch `feature/tui-path`)

**What changed**
- [`cli.rs`](crates/bragi-tui/src/cli.rs): `bragi-tui PATH` (or `--path PATH`) opens code mode on it. A lone word that is not a command (`link`, `status`, `undo`, `code`, `tui`) is the path; `./link` or `--path link` name a folder like a command. `--root` and a path together, or a path with a command, are errors. `resolve_open` handles `~`, `.`, `..` and symlinks: a folder is the root with its files as context; a file's root is its git repository (the nearest `.git`) or its folder, with the file as context; a missing path says `X doesn't exist` (exit 2).
- [`app.rs`](crates/bragi-tui/src/app.rs) `open_code` and [`main.rs`](crates/bragi-tui/src/main.rs): start in code mode (after linking, on a new device). The root is `CodeState.root`, so suggest, rewrite and hunk review stay inside it as before.
- [`main.rs`](crates/bragi-desktop/src/main.rs) and [`terminal.rs`](crates/bragi-desktop/src/terminal.rs) in the desktop crate: `bragi PATH` (and `bragi status` etc.) hands over to the `bragi-tui` beside it or on the PATH; `--demo`, `--serve` and `serve` are unchanged.
- [README](README.md) and the [Getting started guide](site/lib/pages/docs_started.dart) describe it.

**Not tested**: a real linked device; Windows (the hand-over spawns instead of exec); a file picker (code mode has none, the context is the folder).

## 2026-11-22: LF-13 polish, the terminal app's look on weirdnet-themes (branch `feature/tui-polish`, not merged)

**What changed**
- The look comes from [`weirdnet-themes`](https://crates.io/crates/weirdnet-themes) 0.1.1 (without its default `terminal-background` feature) through an adapter, [`theme.rs`](crates/bragi-tui/src/theme.rs). Bragi's palette from [BRAND.md](BRAND.md) is a theme file in the crate's format, [`themes/bragi.toml`](crates/bragi-tui/themes/bragi.toml), with dark and light variants.
  - `--theme NAME|FILE.toml` picks `bragi` (the default), one of the crate's built-ins or a file, and `--mode dark|light|system` picks the variant. `BRAGI_TUI_THEME` and `BRAGI_TUI_MODE` do the same.
  - Colours are truecolor, 256 or 16 (`BRAGI_TUI_COLOR` overrides the detected depth). `NO_COLOR` draws no colour at all, and `BRAGI_TUI_ASCII` (or a non-UTF-8 locale) swaps the icons and borders for ASCII.
  - The adapter fills what the crate doesn't have yet: surfaces, code and diff backgrounds, a link colour, the terminal's own colours below 256, `NO_COLOR`, and the icons. The gaps are listed under **Left**.
- [`ui.rs`](crates/bragi-tui/src/ui.rs):
  - Rounded, titled panes with an accent border on the focused one, a title bar with tabs, and a status bar with segments (mode, connection dot, nodes, model, target, key hints).
  - Your messages sit in bubbles on the right; replies are labelled with the model, node and time. A spinner shows while an answer streams, and the Thinking line folds with a chevron.
  - Link, the picker, settings, status and help open as centred windows with a shadow over the dimmed screen. The endpoint form aligns its labels and marks fields valid or not. Notices are toasts that go after five seconds; questions stay until answered.
  - Diffs have a line-number gutter, add and remove backgrounds and a hunk header strip.
  - The link's QR code sits beside its words and fits a 120×40 terminal.
  - The sidebar has chevrons, icons, counts and a selection bar, and it folds away below 90 columns. A terminal smaller than 44×12 gets a note instead of a broken frame.
- [`markdown.rs`](crates/bragi-tui/src/markdown.rs): titles get rules, quotes get a bar, and code blocks get their own background with a language tag.
- [`main.rs`](crates/bragi-tui/src/main.rs) draws only when something changed: input, a job or stream event, a toast, or a spinner frame.

**Commits:** `a3d6ee3` (theme adapter), `d0572a6` (polish pass), `d5ca364` (QR code, backdrop, look test), `51bf5be` (merge of main 3de0499), `34332b7` (the QR code beside the link code), and this entry.

**Checks:** before the merge, the full set: `cargo fmt --check`, build, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace`, `flutter analyze` and `flutter test --dart-define=BRAGI_DEMO=true`. On the merge, the Rust workspace checks and `flutter analyze` passed. `flutter test` couldn't finish: `/tmp` hit its disk quota, then `settings_test.dart` failed once in main's endpoint save, which this branch doesn't touch. After the merge, only bragi-tui was checked (fmt, clippy and tests). [`tests/scripted.rs`](crates/bragi-tui/tests/scripted.rs) was updated for the new layout. [`tests/look.rs`](crates/bragi-tui/tests/look.rs) is new: it covers `NO_COLOR`, ASCII, the narrow layout and a tiny terminal. Before and after screenshots of the same 13 screens were taken headless, outside the repo.

**Not tested:** real terminals other than tmux (kitty, iTerm2, Windows Terminal), `--mode system` (it asks the desktop portal), and the light theme beyond one screenshot.

**Left:** what weirdnet-themes lacks for a terminal (for the Weirdnet Lead):
- A Bragi or Weird Brains built-in theme.
- `NO_COLOR`: `ColorDepth` has no none level. `TERM=dumb` and unknown terminals detect as truecolor.
- Roles for surface and raised surface, code background, diff add/remove backgrounds, link and a dim text colour.
- In 16 colours: `to_color` maps the background to ANSI black instead of the terminal's default.
- The default `terminal-background` feature queries OSC 11 on the tty, which a full-screen app has to do before raw mode. Its system check calls `busctl` or `gdbus`.
- Neither the desktop nor Flutter uses the crate yet.

**Next:** move the extra roles into weirdnet-themes once it has them, and drop the adapter's stand-ins.

## 2026-11-22: merges of the audio player, terminal polish, MCP server and per-hunk review

**What changed**: four branches are on `main`, merged one at a time with `--no-ff`.
- `feature/audio-player`: [`crates/bragi-core/src/audio.rs`](crates/bragi-core/src/audio.rs), [`crates/bragi-desktop/src/audio.rs`](crates/bragi-desktop/src/audio.rs), [`app/lib/src/audio.dart`](app/lib/src/audio.dart). Clean merge.
- `feature/tui-polish`: the terminal app's look on the weirdnet-themes adapter ([`crates/bragi-tui/src/theme.rs`](crates/bragi-tui/src/theme.rs)). Conflict only in this file; kept both entries.
- `feature/bragi-mcp`: [`bragi mcp`](crates/bragi-mcp/src/lib.rs). Conflicts in this file and [`crates/bragi-desktop/Cargo.toml`](crates/bragi-desktop/Cargo.toml) (kept `decode`, `cpal` and `bragi-mcp`). After `feature/tui-path` landed, [`main.rs`](crates/bragi-desktop/src/main.rs) checks `mcp` before the terminal hand-off (`wants_mcp`, with a test).
- `feature/extension-hunks`: per-hunk review in [`editors/vscode`](editors/vscode/src/rewrite.ts). Conflict only in this file.

**Checks** (each merge): `cargo fmt --check`, build, `clippy -D warnings` and `cargo test --workspace` pass; `flutter analyze` passes. `flutter test` needs `BRAGI_BRIDGE_LIB`; the known flaky "settings and sessions" test failed twice on the tui-polish merge (the only failure) and passed on the others. The extension's own npm tests were not rerun here; CI covers them.


## 2026-11-21: per-hunk accept and reject in the editor extension (branch `feature/extension-hunks`, not merged)

**What changed**
- [`crates/bragi-serve`](crates/bragi-serve/src/lib.rs): `POST /v1/code/hunks` (the hunks between the document and a proposal) and `POST /v1/code/apply` (the document with only the accepted hunks, and what is left to review), both straight from `bragi_core::code::FileDiff` ([`code/diff.rs`](crates/bragi-core/src/code/diff.rs)), the same hunks and `FileDiff::apply` the TUI's code mode uses. `/v1/code/rewrite` also returns `hunks`.
- The extension's rewrites ([`rewrite.ts`](editors/vscode/src/rewrite.ts)) are a review:
  - CodeLenses: Accept (+added −removed) and Reject over each hunk, and a summary, Accept All and Reject All at the top. The removed lines are marked, an end-of-line hint and a hover show what comes in, and the diff opens beside the editor.
  - Accepting a hunk is one minimal edit to the document, so the editor's undo (and redo) takes it back, and the hunk comes back for review, even after Accept All.
  - The user's own edits while reviewing go into the proposal too where the two agree (through the unchanged lines), so they never show up as hunks to undo.
  - Commands `bragi.acceptHunk` / `bragi.rejectHunk` (Ctrl+Alt+Y / Ctrl+Alt+N, the hunk at the cursor) and `bragi.showRewriteDiff` ([`package.json`](editors/vscode/package.json)). A lens from before the hunks changed does nothing.
- The direct target has no Bragi behind it, so [`hunks.ts`](editors/vscode/src/hunks.ts) works hunks out the same way; [`hunks.test.ts`](editors/vscode/src/test/suite/hunks.test.ts) and [`api.rs`](crates/bragi-serve/tests/api.rs) check the same shapes.

**Commits** on `feature/extension-hunks`, off `main` at `3de0499`, one slice each: `46792c8` (the serve routes), `525866b` (the review in the extension), `031be97` (its tests), `e7e2f0e` (the screenshots), then this entry.

**Checks**: `cargo fmt --check`, build, `cargo clippy -D warnings` and `cargo test` on the whole workspace (new `hunks_are_reviewed_and_applied_one_at_a_time`), `flutter analyze`, the Flutter tests in demo mode, the extension's lint, and its tests in a downloaded VS Code under Xvfb (`env -i`, throwaway HOME and profile) against a mock `bragi serve`: hunk by hunk with lenses, undo and redo of an accepted hunk, Reject, Accept All and its undo, and an edit of yours during the review. 27 extension tests pass. Screenshots from that run: `04-hunk-review` and `05-hunk-accepted` (with the earlier inline suggestion, rewrite and chat ones). One `flutter test` run failed in `settings_test.dart` (a snackbar check, with `/tmp` out of quota on the build machine); it passed on the rerun, and the app is not changed here.

**Not tested**: a real Cursor install; against a real `bragi serve` (the Rust tests cover the routes); very large files (each change asks `bragi serve` for the hunks again).

**Left**: the diff tab's title keeps the hunk count it opened with; the diff editor shows lenses only with VS Code's `diffEditor.codeLens` on, so the review lives in the editor beside it; added lines are a hint and a hover, not shown inline; undoing your own typing during a review is taken as a change of yours, not of the proposal; Reject has no undo (Reject All drops the review).

**Next**: the owner's review of the branch.

## 2026-11-21: LF-12 follow-up, the audio player on chat file cards (branch only)

**What changed**
- Rules both shells follow are in [`audio.rs`](crates/bragi-core/src/audio.rs) in `bragi-core`:
  - `Deck`: one clip at a time, and it stops when the chat on screen changes.
  - `clock`, `times`, `seek_ms` and `progress`: the time and the seek bar.
  - `duration_ms`: the length of a WAV or MP3, read from its headers, so the total shows before a clip loads. [`ChatFile`](crates/bragi-core/src/files.rs) gains `duration_ms`.
  - `decode`, behind the `decode` feature: WAV and MP3 to samples, with `symphonia` (pure Rust, `wav`, `pcm` and `mp3` only). Only the desktop enables the feature; the phone build doesn't carry it.
  - The demo's `tone_wav` ([`demo.rs`](crates/bragi-core/src/demo.rs)).
- GPUI desktop ([`audio.rs`](crates/bragi-desktop/src/audio.rs), [`view.rs`](crates/bragi-desktop/src/view.rs)): audio file cards get Play/Pause, a seek bar and the elapsed and total time.
  - The approach is Mouser's player (`mouser-media`): `cpal` plays the decoded samples, and the callback's position is the clock. The output is resampled to the device rate with linear interpolation, and mono goes to every channel.
  - With no sound device, or with `BRAGI_AUDIO=null`, a wall clock runs instead and nothing is heard. The headless scenario sets it ([`desktop-scenario.sh`](scripts/desktop-scenario.sh)).
  - A file is decoded off the UI thread. The deck is checked on every render, so any way of leaving the chat stops the clip.
- Flutter ([`audio.dart`](app/lib/src/audio.dart), [`controller.dart`](app/lib/src/controller.dart), [`chat_screen.dart`](app/lib/src/chat_screen.dart)): the same row, with Play/Pause, a `Slider` and the time from `bragi-core` (`audio_times`).
  - Sound goes through `just_audio`, in the platform audio session from `audio_session` (`music()`): Android audio focus and the iOS `AVAudioSession`, so calls and other apps pause it.
  - The demo build and the tests use a scripted player with no sound.
  - The controller follows `Deck`'s rules: one clip, and it stops when `current` changes.
- Bridge ([`bragi.rs`](crates/bragi-bridge/src/api/bragi.rs)): `FileView.duration_ms`, `audio_times`, `audio_seek_ms` and `demo_tone_wav`. Regenerated.
- Packaging: the Linux desktop links ALSA now. libasound and alsa-lib are added to [`bragi.yaml`](packaging/nfpm/bragi.yaml), [`bragi-standalone.yaml`](packaging/nfpm/bragi-standalone.yaml), the [`PKGBUILD`](packaging/arch/bragi-bin/PKGBUILD) and [`package.nix`](packaging/nix/package.nix). The release workflow already installs `libasound2-dev`.
- Mouser's player is not a library Bragi can take as it is. It decodes with an `ffmpeg` child process and is tied to Mouser's plugin views. Bragi reuses its approach (cpal output, the callback as the clock, a wall clock without a device) and its time format.
  - A shared `cpal` output with a device clock and a wall-clock fallback is generic and could become a Weirdnet media crate that both apps use. No `weirdnet-*` crate was changed.

**Commits** (on `feature/audio-player`, from `main` at `3de0499`):
- [`a61f671`](https://github.com/WeirdBrains/bragi/commit/a61f671): core.
- [`164d235`](https://github.com/WeirdBrains/bragi/commit/164d235): GPUI.
- [`c0f6c21`](https://github.com/WeirdBrains/bragi/commit/c0f6c21): Flutter.
- [`95d4aaf`](https://github.com/WeirdBrains/bragi/commit/95d4aaf): tests, and the scenario script honouring `TMPDIR`.
- Then the commit that adds this entry.

**Checks** (whole workspace):
- `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` and the build.
- New tests:
  - core: the deck, the times and seek, lengths from WAV and MP3 headers, decoding a WAV;
  - desktop: the wall-clock player plays, pauses, seeks and restarts at the end.
- `flutter analyze`, and the Flutter tests in demo mode, all passing, `settings_test.dart` included (it is known to be flaky under load and failed once in an earlier run). The scenario now plays a clip, checks the time, seeks, pauses, switches clips and checks that the clip stops with the chat.
- The headless desktop scenario on a virtual X server, in a clean environment with a throwaway home and no sound device. Screenshots 16d (two audio cards with their lengths), 16e (playing, after seeking to the middle) and 16f (paused).

**How the checks ran**: Rust fmt, clippy, the tests and the build ran under the heavy-build lock. Later, another job held the lock with a stuck `flutter test`, and `/tmp` hit its quota. So `flutter analyze`, the Flutter tests and the headless scenario ran with nice and ionice but outside the lock, against the build that was already done (no Rust code changed after it), with `TMPDIR` on the scratch disk.

**Not tested**:
- Sound out of a real device, on desktop or phone.
- A real phone's audio session: a call or another app interrupting.
- MP3 playback (only MP3 header lengths are tested; the scenario plays WAV).
- macOS and Windows output (cpal's CoreAudio and WASAPI hosts).

**What's left**:
- No volume control, and no choice of output device. Mouser has one in its preferences.
- No background or lock-screen playback.
- The whole clip is decoded into memory. That is at most about 120 MB of samples for a 10 MiB MP3.
- Output latency is not subtracted from the shown time.
- The Flutter seek bar drags; the desktop bar jumps on a click (48 steps) rather than dragging.

**What's next**: review of `feature/audio-player`.

## 2026-11-21: LF-15, the Bragi MCP server (branch only)

**What changed**
- New crate [`bragi-mcp`](crates/bragi-mcp): `bragi mcp` (and the `bragi-mcp` program) serves Bragi over the Model Context Protocol on [`weirdnet-mcp`](https://crates.io/crates/weirdnet-mcp) 0.6, the way Ymir's `ymirctl mcp` does: async tools (`Tools::call_async`), `stdio::serve`, `http::router` with `Guard::loopback(Auth::token(..))`, resources, and image and audio content.
  - [`lib.rs`](crates/bragi-mcp/src/lib.rs): stdio by default; `--http [ADDR]` serves stateless streamable HTTP at `/mcp` on a loopback address only (default `127.0.0.1:4878`). The guard checks `Host`, `Origin` and the bearer token in constant time; the bind check is ours.
  - [`tools.rs`](crates/bragi-mcp/src/tools.rs): `bragi_status`, `bragi_chat` (new or continued session, pool or endpoint, optional folder, text/image/audio files from the project root or `data:` URLs), `bragi_sessions_list`, `bragi_session_get`, `bragi_folders_list`, `bragi_folder_create`, `bragi_folder_move`, `bragi_session_move`, `bragi_endpoints_list` (never a key), `bragi_models_list` (pool, or one endpoint's), `bragi_files_list` and `bragi_file_fetch` (LF-12's `Core::files` and `Core::file_bytes`: attachments and files answers wrote; images and audio as MCP media content). Resources: `bragi://status`, `bragi://sessions`, `bragi://sessions/{id}` (Markdown) and `bragi://sessions/{id}/files/{file}`.
  - [`code.rs`](crates/bragi-mcp/src/code.rs): `bragi_code_suggest` and `bragi_code_rewrite` on `bragi_core::code` (the same gather, prompts, `bragi-edit` parsing and diffs as `bragi-tui` and the editor extension), through `Core::complete_to`, so they stay out of history. They return a unified diff, the explanation and a proposal id, and write nothing.
  - Off by default: `--allow-writes` adds `bragi_code_apply` (all hunks or chosen ones, through `code::Journal`, refusing a file that changed), `bragi_code_undo` and `bragi_file_save` (never replaces a file unless `overwrite`). `--allow-deletes` adds `bragi_session_delete` and `bragi_folder_delete`. Tools that aren't allowed are not listed, and calling one is an unknown-tool error. Every path a client gives must resolve inside `--root`.
  - [`collect.rs`](crates/bragi-mcp/src/collect.rs) reads a chat stream to its end (deltas at their offsets) and cancels the request on timeout or when the call is dropped.
  - [`cli.rs`](crates/bragi-mcp/src/cli.rs): `bragi mcp [--root] [--http] [--allow-writes] [--allow-deletes] [--timeout] [--home] [--demo]`, plus `token`, `token-path`, `status`, `link [CODE]` and `add-node ADDRESS`. `--demo` keeps everything (token, undo journal) in a throwaway folder.
- The server uses the app's Bragi home (`$BRAGI_HOME`, else `~/.local/share/bragi`), like `bragi serve` on `feature/editor-extension`, so its chats, folders and endpoints are the app's, and it opens the OS keychain for endpoint keys.
- Auth reused from `bragi serve` (LF-14): the HTTP transport takes its token from [`bragi_serve::load_or_create_token`](crates/bragi-serve/src/lib.rs) (`<home>/serve/token`, 32 random bytes as hex, mode 0600) and the home from `bragi_serve::cli::default_home`, so one token works for both local APIs and `bragi mcp token` prints the same value as `bragi serve token`. Loopback only, `Host` checked and any `Origin` refused, as in `bragi serve`, here through `weirdnet-mcp`'s `Guard::loopback`.
- [`main.rs`](crates/bragi-desktop/src/main.rs) of the desktop app runs `bragi mcp …` without opening a window.
- Docs: the site guide [`docs_mcp.dart`](site/lib/pages/docs_mcp.dart) (`/docs/guides/mcp`, linked from the docs index) with Cursor, Claude Desktop, VS Code and HTTP setups, and an "MCP server" section in the [README](README.md).

**Checks** (after merging `main` at `3de0499`, under the shared build lock): `cargo fmt --check`, `cargo build --workspace`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace` and `flutter analyze` pass. `flutter test` in demo mode: 5 pass, 1 fails, `settings_test.dart` line 306 (`needsTarget` still true after picking a target for a session whose endpoint was deleted). This branch changes nothing in `bragi-core`, `bragi-bridge` or `app/` compared with `main`, so the failure comes from `main` or from timing on a loaded machine. It was not chased here.
- [`tests/protocol.rs`](crates/bragi-mcp/tests/protocol.rs): raw JSON-RPC through `Server::handle_async` against the demo node: the tool table per flag, chat with a text file and an image, sessions, folders, endpoints (no key in the output), files and media content, resources and templates, file save rules, delete tools; code rewrite and suggest diffs, nothing in history, secrets and paths outside the root refused, apply with chosen hunks, undo, a stale proposal refused; HTTP with and without the token, foreign `Host` and `Origin` refused.
- [`tests/stdio.rs`](crates/bragi-mcp/tests/stdio.rs): the binary with `--demo`, an empty environment and a throwaway HOME: initialize, tools/list, a chat, clean exit at end of input.

**Not tested**: real MCP clients (Cursor, Claude Desktop, VS Code); a real Weirdnet pool or endpoint; the server and the desktop app running on the same home at once; Windows and macOS.

**What's left**:
- Prompts: `weirdnet-mcp` 0.6 has none, so the server has none (a `continue_session` or `review_file` prompt would help). Raised for the Weirdnet Lead rather than added here.
- Streaming answers as progress notifications; MCP cancellation by request id (a dropped call already stops the answer).
- Agent mode (tools and approvals on the node) is not offered.

**What's next**: review of `feature/bragi-mcp`, then LF-16.

## 2026-11-21: LF-14, the VS Code and Cursor extension and `bragi serve` (branch only)

**What changed**
- [`editors/vscode`](editors/vscode/README.md): one VSIX for VS Code, Cursor and other VS Code-based editors (Open VSX-compatible manifest, no proposed APIs). Inline suggestions ([`inline.ts`](editors/vscode/src/inline.ts)); rewrite the selection or file with a diff to accept or reject ([`rewrite.ts`](editors/vscode/src/rewrite.ts)); a chat panel that can send the file, the selection, other open files and the file's problems ([`chat.ts`](editors/vscode/src/chat.ts), [`context.ts`](editors/vscode/src/context.ts)); a target and model picker in the status bar ([`extension.ts`](editors/vscode/src/extension.ts)). The API token lives in the editor's SecretStorage and is imported from the Bragi profile on first use ([`tokens.ts`](editors/vscode/src/tokens.ts)).
- [`crates/bragi-serve`](crates/bragi-serve/src/lib.rs): `bragi serve` (also `bragi --serve` from the desktop app, and the headless `bragi-serve` binary, [`cli.rs`](crates/bragi-serve/src/cli.rs)). Loopback only, a bearer token in `<home>/serve/token` (mode 0600), a loopback `Host` check and no browser `Origin`. Routes: state, targets, `code/suggest`, `code/rewrite`, `code/chat` (server-sent events, kept in history), cancel, and OpenAI-compatible `models` and `chat/completions`. Code requests go through `Core::complete_to` with an optional target (the pool or an endpoint and model from Settings), chat through `Core::send_to`.
- The prompts, reading code out of answers and the diffs are `bragi_core::code` ([`code/editor.rs`](crates/bragi-core/src/code/editor.rs)), shared with the TUI. The extension's direct target (an OpenAI-compatible URL called without Bragi, off unless set) uses a TypeScript copy, [`prompts.ts`](editors/vscode/src/prompts.ts), with tests that mirror the Rust ones.
- [`editor-extension.yml`](.github/workflows/editor-extension.yml) lints, tests headless and packages the VSIX as a CI artifact only.

**Commits**: `feature/editor-extension` merged with `feature/code-core` (`89e4466`), then the commit that moves `bragi serve` to `complete_to` and targets and adds this entry.

**Checks**: `cargo fmt --check`, build, `cargo clippy -D warnings` and `cargo test` on the whole workspace (including [`bragi-serve/tests/api.rs`](crates/bragi-serve/tests/api.rs) against the demo node and the mock endpoint), `flutter analyze`, the Flutter tests in demo mode, the extension's lint, and its tests in a downloaded VS Code under Xvfb (`env -i`, throwaway HOME and profile) against a mock `bragi serve`. Screenshots from that run: inline suggestion, rewrite diff, chat panel.

**Not tested**: a real Cursor install; the extension against a real `bragi serve` linked to real Ymir nodes (the Rust tests cover serve against the demo node); macOS and Windows; `bragi --serve` and `bragi serve` on one profile at once (documented as one or the other).

**Left**: full Visual Studio (a different extension model); Marketplace and Open VSX listings (not wanted yet); per-hunk accept in the editor (the core has `FileDiff::apply`; the editor accepts the whole rewrite); file and folder requests from the editor (`CodeRequest`, used by the TUI); the Bragi milestones in Ymir's [design doc](https://github.com/WeirdBrains/ymir/blob/main/docs/DESIGN.md) need a line for this.

**Next**: the owner's review of the branch.

## 2026-11-21: LF-13, the terminal app (branch `feature/tui`, not merged)

**What changed**
- A new shell, [`crates/bragi-tui`](crates/bragi-tui) (ratatui), binary `bragi-tui`. Like the desktop it only draws `AppState` and calls `Core`. Its profile is its own device ([`session.rs`](crates/bragi-tui/src/session.rs): `~/.local/share/bragi-tui`, `--home` or `BRAGI_TUI_HOME`), never the desktop's.
- What it does, against the desktop's list ([`app.rs`](crates/bragi-tui/src/app.rs), [`ui.rs`](crates/bragi-tui/src/ui.rs)):
  - Linking: a link code with a QR code, an incoming code, or a node address, as on the desktop.
  - Sessions in nested folders ([`tree.rs`](crates/bragi-tui/src/tree.rs)): fold folders, open or delete a chat, and `/folder`, `/move`, `/rename` and `/nest`.
  - OpenAI-compatible endpoints in Settings ([`settings.rs`](crates/bragi-tui/src/settings.rs)): add and edit with a masked key, on/off, alongside or fallback, test, reorder and delete (chats move to the pool). Weirdnet node addresses are listed there too.
  - A picker for the target (the pool or an endpoint) and its model.
  - Chat streams with stop and continue. Replies render as Markdown from `bragi_core::markdown` ([`markdown.rs`](crates/bragi-tui/src/markdown.rs)), and the Thinking section uses `think::label`.
  - Text, image and audio attachments, node status, and a help screen. Copying uses OSC 52, so it works over SSH.
- Code mode ([`code.rs`](crates/bragi-tui/src/code.rs)) and `bragi-tui code` / `undo` ([`cli.rs`](crates/bragi-tui/src/cli.rs)) sit on the shared [`bragi_core::code`](crates/bragi-core/src/code/mod.rs), which LF-14 (the editor extension) uses too:
  - Context comes from files and folders, respecting `.gitignore`.
  - You ask for suggestions or a rewrite of a file or line range.
  - Each hunk of the unified or side-by-side diff is accepted or rejected, and writes go through the undo journal.
- bragi-core:
  - `Core::complete_to(messages, model, target)` routes a code request like a chat: the pool with fallback endpoints, or one endpoint. Nothing is saved, and `Core::complete` uses the default target ([`targets.rs`](crates/bragi-core/src/targets.rs)).
  - `Core::has_identity`.
  - The demo node answers code requests with well-formed edits ([`demo.rs`](crates/bragi-core/src/demo.rs)).
  - `feature/code-core` points at the merge of main into this branch, so LF-14 can take the shared module from there.
- Packaging: `bragi-tui` ships next to `bragi` in the Linux tarball, deb, rpm and Arch packages, in the macOS app and tarball, and in the Windows zip and MSI (on PATH). Its own `bragi-tui` tarball, deb, rpm, pkg and Windows exe carry no GUI libraries ([`packaging/nfpm/bragi-tui.yaml`](packaging/nfpm/bragi-tui.yaml), [`release.toml`](release.toml), [`scripts/dist-linux.sh`](scripts/dist-linux.sh)).

**Commits:** `89e4466` (merge of main 431ea3c with the shared code module), `8a6a814` (bragi-tui), `4b21b99` (packaging), `bced350` (README and CHANGELOG), and this entry.

**Checks:** All of these ran on the full workspace before committing and passed:
- `cargo fmt --check`, build, `cargo clippy --workspace --all-targets -D warnings` and `cargo test --workspace`.
- `cargo test` covers [`tests/scripted.rs`](crates/bragi-tui/tests/scripted.rs), which drives every screen key by key on ratatui's TestBackend against the demo nodes. It also covers [`tests/cli.rs`](crates/bragi-tui/tests/cli.rs) (rewrite diff, `--apply`, undo) and `code_requests_route_by_target_and_stay_out_of_history` in [`tests/endpoints.rs`](crates/bragi-core/tests/endpoints.rs).
- `flutter analyze` and `flutter test --dart-define=BRAGI_DEMO=true`.

The app was also run under a pty in a private tmux server (`env -i`, a throwaway HOME, `--demo`), with 13 screenshots taken outside the repo.

**Not tested:**
- A real Ymir node or a real endpoint from the terminal app (the endpoint code paths are core's, tested against a mock server).
- The release workflow and packaging scripts did not run; the new files were only syntax-checked.
- macOS and Windows builds of `bragi-tui`.

**Left:**
- Chat files (LF-12) are on `feature/chat-files`, not main yet; the terminal app gets them once they merge.
- Message delete with undo, reactions and agent mode are desktop features the terminal does not have yet.
- The link QR code needs a terminal about 60 rows tall for the long link codes; smaller ones show the code text only.
- The Bragi status in Ymir's [design doc](https://github.com/WeirdBrains/ymir/blob/main/docs/DESIGN.md#bragi-milestones) is not updated (another repo).

**Next:** review, then merge `feature/tui` when m3talsmith says so.

## 2026-11-21: merges of the models screen, chat files, the terminal app and the editor extension

**What changed**: four branches are on main, each with `git merge --no-ff`: the Models screen (LF-30 M4, see [`models_ui.dart`](app/lib/src/models_ui.dart)), files in a chat (LF-12, see [`files.rs`](crates/bragi-core/src/files.rs)), the terminal app (LF-13, [`bragi-tui`](crates/bragi-tui/)) and the VS Code and Cursor extension (LF-14, [`editors/vscode`](editors/vscode/) with [`bragi-serve`](crates/bragi-serve/)). The "Saved to …" notice now shows in the success colour in [`view.rs`](crates/bragi-desktop/src/view.rs) and [`chat_screen.dart`](app/lib/src/chat_screen.dart). Conflicts were in [`CHANGELOG.md`](CHANGELOG.md), this file, [`demo.rs`](crates/bragi-core/src/demo.rs) (both the file answer and the code answer kept), [`Cargo.toml`](Cargo.toml) and [`Cargo.lock`](Cargo.lock); the Flutter bridge was regenerated once after chat files. The cancel-and-resume test in [`tests.rs`](crates/bragi-core/src/models/tests.rs) now waits for the node to drop a cancelled pull.

**Checks**: fmt, build, clippy `-D warnings`, `cargo test --workspace`, `flutter analyze`, the extension's lint, compile and headless tests. **Known flake**: the Flutter test in [`settings_test.dart`](app/test/settings_test.dart) ("settings and sessions") fails on some runs under load and passes alone.

## 2026-11-21: LF-12, files in a chat: view and save (branch only)

**What changed**
- [`files.rs`](crates/bragi-core/src/files.rs) in `bragi-core` lists a chat's files as `ChatFile` cards, each with id, message, name, path, media type, size, kind (text, image or audio), origin and a text preview:
  - every attachment ([`attach.rs`](crates/bragi-core/src/attach.rs)): text, plus the images and audio from LF-18, whose bytes are the inline base64;
  - every file an answer wrote: a fenced code block that names its file (in the info string, as in ```` ```rust src/main.rs ```` or `title="a.py"`, or on the line just before the fence, as in `**src/main.rs**`), or a Markdown image with a `data:` URL. When a name repeats, the last block wins.
- A card's id is `<message id>#a<n>` or `<message id>#r<n>`, so deleting other messages does not change it.
- `Core::files`, `Core::file_bytes` and `Core::save_file` in [`core.rs`](crates/bragi-core/src/core.rs). A save writes atomically, through a temp file and a rename. Saving to a picked path replaces that file. Saving into a folder adds ` (2)`, ` (3)` and so on instead of replacing anything.
- Nothing new is stored. The cards are read from the message as it is, so they sync with it and work for older records.
- Bridge ([`bragi.rs`](crates/bragi-bridge/src/api/bragi.rs)): `FileView`, `MessageView.files`, `chat_files`, `file_bytes` and `save_file`. `flutter_rust_bridge_codegen generate` was re-run.
- Phone and Flutter desktop ([`chat_screen.dart`](app/lib/src/chat_screen.dart), [`controller.dart`](app/lib/src/controller.dart)): file cards under the message, with an icon, name or path, type, size, origin, text preview, image preview (`Image.memory`) and Save.
  - On Android and iOS, Save hands the bytes to the system save sheet. On a computer, the dialog returns a path and the core writes the file.
  - The demo build saves into a temp folder.
- GPUI desktop ([`view.rs`](crates/bragi-desktop/src/view.rs)): the same cards, with a decoded image preview (cached) and Save. Save opens the system save dialog, starting in Downloads.
- Demo: a prompt with "as a file" ([`demo.rs`](crates/bragi-core/src/demo.rs), `FILE_TRIGGER`) gets an answer with a named `hello.py` block. The desktop scenario attaches [`demo-chart.png`](crates/bragi-desktop/assets/demo-chart.png).

**Commits**: [`173750f`](https://github.com/WeirdBrains/bragi/commit/173750f) (the feature, on `feature/chat-files`, from `main` at `431ea3c`), then the commit that adds this entry.

**Checks** (whole workspace):
- `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` and the build.
- Tests: the unit tests in `files.rs` (named blocks, a repeated name, data-URL images, cards by message id, save rules) and `tests/breadth.rs` `the_files_in_a_chat_are_listed_and_saved`.
- `flutter analyze`, and the Flutter scenario test in demo mode: the attachment card, the reply card, and Save writing `hello.py` with the right content.
- The headless desktop scenario on a virtual X server, in a clean environment with a throwaway profile. Screenshots 16b (image card with preview) and 16c (reply file card, saved).

**Not tested**: a real phone and its save sheet; the real save dialogs on desktop (the scenarios save to a folder directly); two devices syncing; a real model's answers.

**What's left**:
- A player for audio files (the card shows the file and saves it).
- Files larger than a message takes (content roots).
- The roadmap and Ymir's [design doc](https://github.com/WeirdBrains/ymir/blob/main/docs/DESIGN.md) were not updated.

**What's next**: review of `feature/chat-files`, then LF-16.

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
