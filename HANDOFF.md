# Handoff log

A running record of finished work on Bragi, newest first. Each entry says what changed, the commits, which checks ran and which did not, what is left and what comes next. Owner: [m3talsmith](https://github.com/m3talsmith). See also [ROADMAP.md](ROADMAP.md), [CHANGELOG.md](CHANGELOG.md) and [AGENTS.md](AGENTS.md).

## 2026-11-26 17:40 EST: video V2 slice 2, the inline player and the terminal app's frames and play (merged `feature/video-v2-player`, LF-43)

The rest of V2 in [docs/design/video.md](docs/design/video.md), on top of slice 1 (merge 54d6a27, entry below).

- New [video_play.rs](crates/bragi-core/src/video_play.rs): `Fallback` (`NotHere`, `NoDecoder`, `CantDecode`, `TooSlow`, each with what to tell the user), `playable_clip`, `open_externally`/`system_open_command` (`xdg-open`, `open`, `cmd /c start`; video files only), `JpegSplitter`, `FrameQueue` (follows the clock, drops late frames, `too_slow` measured over a 3 s grace at 1.5 s behind), `decode_shape`/`decode_args` (at most 15 fps and 960 px, MJPEG over `image2pipe`, the same hardened input flags as sampling), `Decoder` (ffmpeg child killed on drop, 24 frames ahead), `soundtrack`, `silence`, `bubble`, `frame_list`, `last_video`.
- `Sampled.clip` (optional, skipped when empty): the original clip's path on the device that attached it, set when `keep_original` is on (the default). It is not copied into the attachment store, so another device says the clip isn't here and plays nothing.
- Desktop ([video.rs](crates/bragi-desktop/src/video.rs), [view.rs](crates/bragi-desktop/src/view.rs)): `VideoPlayback` uses the audio `Player` as the master clock (a silent clock when the clip has no sound), with the picture following it. Each video attachment gets a card showing the bubble line, the picture, Play/Pause, a seek bar, elapsed / total, Mute (it reopens at the same place), **Open in system player**, and the frames the model saw (a click plays from there). When the clip can't play inline, the card says why. Video and audio share the `Deck`, so only one plays at a time, and changing chats stops it. The duplicate file card is hidden for clips.
- Terminal app: the attachment line shows the bubble plus `· /frames · /play`. `/frames` lists the times and labels sent; `/play` opens the clip in the system player.

**Checks:** on alien-blue: `cargo fmt --all --check` was clean at d3086c5. `cargo clippy --workspace --all-targets -- -D warnings` was clean at 32bd1f8, and later commits touched only docs and HANDOFF. `cargo test --workspace --no-fail-fast` at d3086c5 gave 356 passed, 0 failed, 5 ignored in 43 suites. That includes 9 new tests, 5 in `video_play` and 4 desktop `VideoPlayback` tests over a scripted decoder: clock-following and seek-restart, decoder failure, too slow after the grace period, and mute keeping its place. `flutter analyze` showed only the 6 existing `curly_braces` infos. `flutter test --dart-define=BRAGI_DEMO=true` was run file by file against the bridge built from this branch: code 3/3, models 3/3, scenario 2/2, theme 1/1 and wake 3/3 passed. settings failed 0/1, at line 228 on one run and line 389 on the next, in the endpoint flows. It is the same test that fails on origin/main (V1 entry), and this branch changes nothing in `app/` or `bragi-bridge`. Slice 1 checks are in the entry below (54d6a27: fmt, clippy, 347/0/5 in 43 suites). No GUI was opened, so the player was tested by unit tests only and never watched on screen.

**Not done:** the VS Code attach and webview player, the Flutter phone player and decoders (V3), copying the original clip into the attachment store so other devices can play it, fullscreen, a bundled LGPL ffmpeg, wiring transcription once Ymir milestone 2 lands, the Bragi status in Ymir `docs/DESIGN.md`, and the Weirdnet asks (`McpSource` passing image and audio blocks to the model, image-block annotations, `weirdnet-media`). weirdnet-model 0.5.4 is published and not adopted yet.

**Next:** look at the player on a real screen, then VS Code or V3.

## 2026-11-26 17:15 EST: video V2 slice 1, MCP video tools, override UI and endpoint limits (merged `feature/video-v2-mcp-override`, LF-43)

Michael approved rolling straight into V2 of [docs/design/video.md](docs/design/video.md). weirdnet-mcp requirement 0.6 → 0.6.2 (image and audio content blocks, `StdioTransport::spawn_limited`).

- **MCP servers Bragi talks to:** stdio servers start with `spawn_limited(cmd, MAX_STDIO_LINE)` (64 MiB) instead of the 4 MiB default, so a reply carrying frames fits ([mcp_servers.rs](crates/bragi-core/src/mcp_servers.rs)). `McpSource::call` still hands only text to the model (Weirdnet ask).
- **bragi-mcp** ([video.rs](crates/bragi-mcp/src/video.rs)): `bragi_video_probe` (file → duration, size, fps, sound, codecs) and `bragi_video_frames` (file; an explicit `count` 1–64 with `max_edge`, or frames sized from a `model`/`endpoint`'s reported limits; `mode` uniform/scene/hybrid, `start_ms`/`end_ms`). `Content::Image` has no annotations, so each image is preceded by a text block with its time (`[00:07.5]`) and `structuredContent` lists `{index, at_ms, label, mime_type}` per frame plus the clip. A file is a path inside the server root or a `data:` URL. `bragi_chat` takes `video` (+ `video_max_frames`, `video_max_edge`, `video_mode`, `video_start_ms`, `video_end_ms`), counted with the other attachments. Core: `extract` (caller-sized, capped at 64 frames / 32 MiB) and `clip_range`.
- **Endpoint limits:** `video_plan(target, model)`: an OpenAI endpoint target uses `endpoint_video_limits` (its `/v1/models` `weirdnet.*` plus the override; no sound, since an endpoint doesn't say it takes audio); a pool target uses the catalog. `attach_video_source` is the shared path.
- **Override UI:** `ModelLimits::parse/to_text/describe`, `LimitsView` (reported, yours, effective; `chip()`, `ignored()` names the values that would loosen a reported limit). TUI: `/video-limits` (show), `/video-limits image_tokens=258 max_images=32` (fill or tighten), `/video-limits clear`; the composer title shows the chip; attaching runs off the UI thread ("reading frames…"). Desktop: chip under the composer with a Limits editor (same `key=value` text, Save, Reset); attach runs in the background.

**Checks:** on alien-blue at 54529a0: `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace --no-fail-fast` 347 passed, 0 failed, 5 ignored in 43 suites (new: `extract_takes_the_callers_count_and_size_and_refuses_too_much`, `the_override_text_parses_round_trips_and_says_what_is_ignored`, bragi-mcp `probe_frames_and_chat_with_a_clip`, `an_mcp_client_reads_frames_as_images_with_their_times`, which reads frames through `Client::call_tool_content`/`Content::from_json`/`decoded_bytes` over a pipe and shows a 128 KiB line cap failing where 64 MiB delivers). `flutter analyze`: the 6 existing `curly_braces` infos only (no Dart or bridge changes, so no `flutter test`). No GUI opened.

**Not done:** the Bragi status line in Ymir `docs/DESIGN.md` (follow-up), MCP image/audio parts reaching the model through `McpSource` (Weirdnet ask), image-block annotations (Weirdnet ask).

**Next:** the V2 player (`feature/video-v2-player`).

## 2026-11-26 15:10 EST: video V1, sampling clips into frames sized from reported limits (merged `feature/video-upload`, LF-43)

Michael (11-26): "Let's do the video uploads". Phase 1 of [docs/design/video.md](docs/design/video.md), core first. weirdnet-agent 0.5.6 → 0.5.7 (`MediaLimits`, `CatalogModel::limits`), weirdnet-backends 0.4.7 → 0.4.8. Ymir (merged the same day) publishes context and limits in catalog/1.

- New [video.rs](crates/bragi-core/src/video.rs): `ModelLimits` (context plus weirdnet-agent 0.5.7 `MediaLimits`, absent when unknown; frame cost from `image_cost` after scaling to `image_edge`, so patch-priced models get cheaper frames when smaller), `with_override` (fills gaps, only tightens: `MediaLimits::tightest`), `LimitOverrides` (`video-limits.json` in the device folder), `Budget::from_limits` (frames = min(max_images, prefs, fps, (context - a quarter for the reply - history - audio) / image cost); refuses with `Refusal::MissingLimits` naming what is missing, or `NoRoom` under 2 frames for a clip over 2 s), `plan_frames` (uniform, scene, hybrid; sorted, unique, inside the range), `fit`, labels `[00:07.5]`, `sample` (probe, budget, plan, extract, request-size trim: drop frames evenly, then the sound), `FrameSource` with `FfmpegCli` (`-protocol_whitelist file,pipe`, `file:` input, no stdin, time limit and kill, refused containers: concat, hls, image2 and similar, cover-art-only files) and `FakeSource`. Marked for `weirdnet-media`.
- [attach.rs](crates/bragi-core/src/attach.rs): `Media::Video(Sampled)`; parts are the summary, then label + image per frame, then the WAV; `needs` is VISION (+AUDIO when sound goes); one video per message; `from_video`; `from_media` kind `video` takes the JSON. [files.rs](crates/bragi-core/src/files.rs) lists a video by its first frame. Bridge `AttachmentView` kind `video` carries the JSON.
- [video_core.rs](crates/bragi-core/src/video_core.rs): `Core::reported_video_limits` (context and `MediaLimits` from the catalog), `endpoint_video_limits` (an endpoint's `/v1/models` `weirdnet.*`), `attach_video_with`, `video_override`/`set_video_override`, `video_limits`, `video_unavailable`, `attach_video`.
- Shells: `/attach clip.mp4` in the TUI and the desktop attach box route video extensions to `attach_video` (runs in place; off-thread with progress is V2).
- [catalog.rs](crates/bragi-core/src/catalog.rs): `ModelInfo::limits` (strictest across pool entries), `CatalogView::limits_for`. [openai.rs](crates/bragi-core/src/openai.rs): `parse_model_limits`/`model_limits` read `weirdnet.context`, `weirdnet.limits` and `weirdnet.max_request_bytes` (the smaller of the two) from `/v1/models`; a plain OpenAI server reports nothing and gets nothing invented.

**Checks:** on alien-blue: `cargo fmt --all --check`; clippy `--workspace --all-targets -D warnings` (fe4cd53, rechecked after the main merge and 61b289a); `cargo test --workspace --no-fail-fast` 343 passed, 0 failed, 5 ignored in 42 suites (20 video and endpoint-limit tests among them; the `#[ignore]` ffmpeg test not run). `scene_output_is_parsed` caught ffmpeg seconds being truncated to ms (1.001 s read as 1000 ms); times are rounded now (61b289a). `flutter analyze`: only the 6 existing `curly_braces` infos (no Dart changes). `flutter test --dart-define=BRAGI_DEMO=true` with the built bridge, file by file: code 3/3, models 3/3, scenario 2/2, theme 1/1, wake 3/3; settings 0/1 fails at `settings_test.dart:225` (saved endpoint list empty), the same on origin/main e146c5a with the same bridge, so it isn't this branch (models and theme failed once while `cargo test` was relinking the bridge, then passed).

**Not done:** settings sheet and chip (override UI) in each shell, the TUI and desktop using `endpoint_video_limits` when the target is an endpoint (they use the pool catalog plus override today), `bragi-mcp` `video` input with `video_probe`/`video_frames`, GPUI inline player with system-player fallback (V2), TUI frame list and open-externally, VS Code attach and `<video>`, Flutter phone decoders and player (V3), transcription, bundled LGPL ffmpeg, sampling off the UI thread with progress.

**Next:** override UI in each shell, then the MCP tools and the V2 player.

## 2026-11-26 13:05 EST: v0.15.1 released

**Release:** [v0.15.1](https://github.com/WeirdBrains/bragi/releases/tag/v0.15.1) (`release: v0.15.1`, 5e33552, patch cut on main b4383e7; Release run 45863593860, green after re-running Android, which first hit Maven Central `429 Too Many Requests`): one sync session per device (weirdnet-sync 0.8.5 `pull_types_shared`) and the delete test waiting for the disk. GitHub: 41 files, leak scan 0 hits (`pub-bragi.sh`, assets ok), then the 6 Arch `.sig`s (`sign-bragi.sh 0.15.1` EXIT 0); 48 assets. Packaging ac93004 on main (bragi-bin PKGBUILD/.SRCINFO, Nix `release.json`, site `latestVersion`); TAUR bragi-bin 0.15.1-1 (dry-run first). Straight to main (no PRs). No live nodes touched.

**Next:** nothing on Bragi that doesn't need Michael (F-Droid key and submission, store listings, APNs/FCM and a push server, B7 on real nodes).

## 2026-11-26 10:15 EST: one sync session per device (weirdnet-sync 0.8.5, on main)

**What:** `Identity::tick` pulls `SYNC_TYPE`, `CONTEXT_TYPE` and (when due, full devices only) the wake cards with `weirdnet_sync::pull_types_shared` in one session per device, instead of three `pull_from_shared` calls (three Hellos and roster swaps). A refused type fails alone; a failed roster pull still marks the pass incomplete, so conversations are not re-sealed. The wire is unchanged (0.7.1 to 0.8.4 peers answer it). weirdnet-sync 0.8.4 → 0.8.5 (Weirdnet Lead's release for this).

**Checks:** fmt, clippy `-D warnings`, `cargo test --workspace` (42 suites ok) on the box. Committed straight to main (Michael: no PRs). No release cut; it goes out with the next one. No live nodes touched.

**Next:** none for this; the next Bragi release carries it.

## 2026-11-26 08:10 EST: v0.15.0 released

**Release:** [v0.15.0](https://github.com/WeirdBrains/bragi/releases/tag/v0.15.0) (`release: v0.15.0`, `f804600`, cut on main `1cea2e8`; app version `0.15.0+18`): B4 wake-ups (weirdnet-push 0.1.1) and the F-Droid recipe. Release run 45820146922: every build green on the first attempt; the `check` job failed once on the delete test's disk race (fixed in `fix/delete-test-disk-race`) and passed on a re-run of that job, no new tag. The leak scan of dist found nothing (41 files, 0 hits), `assets` ok; the six Arch packages are gpg-signed and their signatures uploaded (48 files). This branch (`release/0.15.0-packaging`) moves the bragi-bin PKGBUILD and .SRCINFO, the Nix release.json and the site's `latestVersion` to 0.15.0 (`taur-check` ready). F-Droid: the recipe's v0.15.0 builds are versionCode 1018/2018/4018 (`VercodeOperation` on build 18), matching the tag.

**Upgrade note:** a wake card reaches the user's other devices only once they run 0.15.0; Ymir's side (waking a Bragi device from a seed holder) is on Ymir's `feature/push-wake`.

## 2026-11-26 07:55 EST: delete test waits for the disk (branch `fix/delete-test-disk-race`)

The v0.15.0 Release run's `check` job failed once on Linux: [crates/bragi-core/tests/delete.rs](crates/bragi-core/tests/delete.rs) read B's log from disk right after B's history showed the conversation, but a background sync pass saves the log a moment after it merges. The test now waits (inside `sync_until`) for the disk as well as the history, after the first sync, the delete and the clear. Test-only; the job was re-run on the tag. Checks: clippy `-D warnings`, the delete test 3 of 3 locally.

## 2026-11-26 05:50 EST: push wake-ups, Bragi side (branch `feature/push-wake`)

B4 wake-ups on weirdnet-push 0.1.1 (`relay-wake` feature only). [crates/bragi-core/src/wake.rs](crates/bragi-core/src/wake.rs): per pool, a wake key and handle (`wake.key`, private file, made on first use), the settings (`push` on by default, `relay_wake` off by default, `RELAY_WAKE_COST` the text every shell shows before turning it on) and the guard (`ReplayGuard`, 600 s window: another handle, a bad seal, a repeat or a stale wake is refused and counted). Push off removes `wake.key` (that exact path) and withdraws the card. [crates/bragi-core/src/wake_core.rs](crates/bragi-core/src/wake_core.rs): the card is a sealed record of its own type, `bragi.wake`, so a cert-only server (granted `ymir.context` only) can't read it, and it is pulled only from full devices; `relay_loop` keeps a `/weirdnet/wake/1` subscription to the home relay while relay wake is on (backoff, an idle timeout of two keepalives) and runs a sync pass on every valid wake; `Core::wake_device` seals and delivers a wake to a linked device through its relay. [crates/bragi-core/src/demo_wake.rs](crates/bragi-core/src/demo_wake.rs) is a stand-in relay on the memory transport for tests and the demo.

**Shells:** Flutter Settings, Wake-ups ([app/lib/src/wake_ui.dart](app/lib/src/wake_ui.dart)), the relay wake confirm shows the battery cost; on Android [WakeService.kt](app/android/app/src/main/kotlin/org/weirdbrains/bragi/WakeService.kt) is a foreground service (`remoteMessaging`, a min-importance notification) started and stopped with the switch and at launch ([app/lib/src/wake_service.dart](app/lib/src/wake_service.dart)), so F-Droid builds have a wake path without FCM. Desktop Settings, Wake-ups ([crates/bragi-desktop/src/view/wake.rs](crates/bragi-desktop/src/view/wake.rs)) with the same confirm; `bragi-tui wake [status|on|off|relay on|off] [--yes]` ([crates/bragi-tui/src/wake_cli.rs](crates/bragi-tui/src/wake_cli.rs)). Bridge: `wake_status`, `set_wake_settings`, `relay_wake_cost`, `wakeable_devices`, `wake_device`.

**Tests:** wake.rs unit tests (defaults, push off forgets the key, the guard refuses a repeat, a stale wake and another handle); [crates/bragi-core/tests/wake.rs](crates/bragi-core/tests/wake.rs) end to end on the stand-in relay (defaults, relay connect, the card syncs, a wake is delivered and runs a sync, push off withdraws the card and closes the connection); [app/test/wake_test.dart](app/test/wake_test.dart) (defaults, Cancel keeps relay wake off, push off turns it off); the desktop scenario steps through the confirm (screenshot `30a-relay-wake-confirm`); `wake_cli` parser tests.

**CI fix:** Platforms on Windows hit the delete test's 10 s undo window (a sync pass now pulls wake cards too, so it is a little slower): [crates/bragi-core/tests/delete.rs](crates/bragi-core/tests/delete.rs) undoes right after the delete reaches B, then deletes again before B's next turn. That was not enough (Windows failed again): each pull of a record type is a sync session of its own (~0.4 s here), so pulling wake cards every pass doubled a pass's cost. Wake cards now come in from each device every `WAKE_PULL_EVERY` (2 min, [crates/bragi-core/src/identity.rs](crates/bragi-core/src/identity.rs)) and again right after this device's wake settings change. The delete test on the branch is still slower locally than on main (about 26 s against 13–20 s, with other load on the machine), not explained yet. Pulling several record types in one weirdnet-sync session would need a crate change.

**Not done / needs Michael:** APNs and FCM delivery (push service accounts and a push server); whether the public relay serves `/weirdnet/wake/1`; Ymir's side (sending wakes, and keeping `bragi.wake` out of cert-only grants). No real-relay check.

## 2026-11-26 05:40 EST: v0.14.0 released

**Release:** [v0.14.0](https://github.com/WeirdBrains/bragi/releases/tag/v0.14.0) (`release: v0.14.0`, `92e6a4a`, cut on main `9faede0`; the cut's fmt, clippy and workspace tests passed): weirdnet-sync 0.8.4, so conversations are shared with Ymir inference servers linked cert-only (Ymir 0.18.0), and chats from before the link are re-sealed for them. Release run 45796114507 green on the first try. 41 files, the leak scan of dist found nothing (41 files, 0 hits), `assets` ok; the six Arch packages are gpg-signed and their signatures uploaded (48 files). This branch (`release/0.14.0-packaging`) moves the bragi-bin PKGBUILD and .SRCINFO, the Nix release.json and the site's `latestVersion` (and `latestDate`) to 0.14.0 (`taur-check` ready).

**Upgrade note:** every Bragi that shares conversations with a cert-only inference server needs 0.14.0; 0.13.0 and older can't open records sealed under the type-scoped key.

TAUR has bragi-bin 0.14.0-1.

**Next:** merge `feature/fdroid-prep` (B3), then the next Bragi batch.
## 2026-11-26 05:10 EST: F-Droid recipe (branch `feature/fdroid-prep`)

B3: the F-Droid draft recipe made submittable. [packaging/fdroid/org.weirdbrains.bragi.yml](packaging/fdroid/org.weirdbrains.bragi.yml) now builds one apk per ABI from v0.15.0 (versionCode 1018/2018/4018, `VercodeOperation`), with Rust 1.99 from the `rustup` srclib, Flutter 3.50.0, `scandelete` of the pub cache and tag-based auto-updates; store text and a 512 px icon are in [fastlane/metadata/android/en-US](fastlane/metadata/android/en-US). mobile_scanner reads QR codes with Google ML Kit (non-free, F-Droid refuses it), so the QR camera moved to [app/lib/src/scanner.dart](app/lib/src/scanner.dart) and F-Droid's prebuild copies [app/lib/src/scanner_none.dart](app/lib/src/scanner_none.dart) over it and drops mobile_scanner from `pubspec.yaml`: pairing with a desktop is by typed code there. [docs/RELEASES.md](docs/RELEASES.md) has an F-Droid section.

**Checks:** `fdroid lint` and `fdroid rewritemeta` clean (fdroidserver 2.4.6 in a venv, fdroiddata's categories); the F-Droid variant simulated in a copy (swap + sed, `flutter pub get` leaves no mobile_scanner in the lock, `flutter analyze` only the existing infos, `flutter test --dart-define=BRAGI_DEMO=true` 10 passed); the normal app the same (10 passed). Not run: `fdroid build` (no Android SDK build server here) or an apk build of the F-Droid variant.

**Needs Michael:** submitting to fdroiddata (a GitLab account and merge request), and whether F-Droid signs with its key or reproducible builds against a WeirdBrains release key (no Android release key in the repository secrets yet).

## 2026-11-26 04:45 EST: weirdnet-sync 0.8.4 (branch `chore/weirdnet-sync-0.8.4`)

Ymir 0.18.0 gives inference nodes linked cert-only a key for `ymir.context` alone (weirdnet-sync 0.8.3/0.8.4): full devices seal conversations under that type-scoped key while such a server is granted the type, and under the epoch key otherwise. Devices on weirdnet-sync 0.8.3 or older can't open type-scoped records, so every Bragi that shares conversations with one needs this.

**What changed:** weirdnet-sync 0.7.1 → 0.8.4, weirdnet-net =0.8.2 → =0.8.3, weirdnet-agent 0.5.4 → 0.5.6, weirdnet-backends 0.4.5 → 0.4.7 (`cargo tree`: one copy of each). `SyncState` gains `voter: None`. New `Identity::reseal_context` ([crates/bragi-core/src/identity.rs](crates/bragi-core/src/identity.rs)), `reseal_stale("ymir.context")` on a full device, run at the end of every sync pass whose context pulls reached every other device (so the re-sealed copy is the latest): conversations from before the server link move to the scoped key, and back to the epoch key once no such server is left. The site's getting-started page says so; CHANGELOG under Unreleased.

**Checks:** `cargo fmt --all --check`; clippy `--locked --workspace --all-targets -D warnings`; `cargo test -q --locked --workspace` all passed (new `conversations_from_before_a_server_link_are_resealed_for_it`: written before the link unreadable on the service device, readable after `reseal_context`, written after the link readable at once, revocation moves both back); `flutter analyze` (only the existing infos); `flutter test --dart-define=BRAGI_DEMO=true` with the bridge library, 10 passed. Platforms and Pages on the branch.

**Not done:** no check against a real Ymir 0.18.0 inference node (B7's real-node check). Ymir's [docs/DESIGN.md](https://github.com/WeirdBrains/ymir/blob/main/docs/DESIGN.md) Bragi status waits until Ymir main is free.

## 2026-11-26 00:55 EST: v0.13.0 released

**Release:** [v0.13.0](https://github.com/WeirdBrains/bragi/releases/tag/v0.13.0) (`release: v0.13.0`, `2b7e317`, cut on main `f6a002c`; the cut's fmt, clippy and workspace tests passed): the Training view on the Models screen in both shells (needs Ymir 0.16.0 on the model nodes). Release run 45760743281 is green after re-running two failed jobs once: `check` (`tests/pools.rs:177`, `rename_remove_and_cancel_a_new_pool`, "Directory not empty" when removing the pool folder) and `macos` (`hdiutil create` ran out of space on the runner); neither touches the training view, and both passed on the re-run. 42 files and `assets` ok; the leak scan of dist found nothing (41 files, 0 hits). The six Arch packages are gpg-signed ("Good signature" each, sums matching) and their signatures uploaded: 48 files. This branch (`release/0.13.0-packaging`) moves the bragi-bin PKGBUILD and .SRCINFO, the Nix release.json and the site's `latestVersion` to 0.13.0 (`taur-check` ready, check_links ok).

TAUR has bragi-bin 0.13.0-1.

**Not fixed:** the `pools.rs` removal race (a background writer in the pool folder while it is removed); seen once on Linux CI.

**Next:** the Training view against a real Ymir 0.16.0 node.

## 2026-11-25 22:55 EST: v0.12.0 released

**Release:** [v0.12.0](https://github.com/WeirdBrains/bragi/releases/tag/v0.12.0) (Release run 45749409093, green): the model upload dialog in the Flutter app. 42 files; the assets check passes; the leak scan of dist found nothing (41 files, 0 hits). The six Arch packages are gpg-signed and their signatures uploaded. TAUR has bragi-bin 0.12.0-1. This branch (`release/0.12.0-packaging`) moves the bragi-bin PKGBUILD and .SRCINFO, the Nix release.json and the site's `latestVersion` to 0.12.0 (`taur-check` ready, check_links ok).

**Next:** Ymir 0.16.0 (model-upload U9) once its CI is green.

## 2026-11-25 22:18 EST: the training view (branch `feature/training-view`)

Ymir's training plan, after Bragi v1: a Training toggle on the desktop Models screen. [crates/bragi-core/src/models/ymir_train.rs](crates/bragi-core/src/models/ymir_train.rs) (TEMPORARY mirror of Ymir's `/ymir/train-status/1`, Ymir 0.16.0: `TrainStatus`, `RunSummary`, `JobSummary`) and [crates/bragi-core/src/models/training.rs](crates/bragi-core/src/models/training.rs): `overview` reads every roster device's training status and rollouts (`ymir_rollouts::read`, new), gives each version its share and `Stage` (live, canary: the newest version with a share beside an older live one, standby) and the node and run whose adapter it serves. `Core::training_overview`. The demo nodes serve a stand-in with one finished run. Desktop: [crates/bragi-desktop/src/view/training.rs](crates/bragi-desktop/src/view/training.rs), Models / Training / Refresh buttons at the top of the Models screen. Flutter: `training_overview` and its views in [crates/bragi-bridge/src/api/bragi.rs](crates/bragi-bridge/src/api/bragi.rs) (`flutter_rust_bridge_codegen generate` re-run), [app/lib/src/training_view_ui.dart](app/lib/src/training_view_ui.dart) behind a Training button on the Models screen; the desktop scenario's step 60 opens the toggle. Main (the Flutter upload) merged in. CHANGELOG; ROADMAP, roadmap.toml and the site roadmap text.

**Checks:** `cargo fmt --check`; clippy `-p bragi-core -p bragi-desktop -p bragi-bridge --all-targets -D warnings`; `flutter analyze` (only the existing `curly_braces` infos); `flutter test --dart-define=BRAGI_DEMO=true` all 10 passed, the models test now opening the Training view (the demo runs, and the upload's rollout); `cargo test -p bragi-core -p bragi-desktop --locked` all passed, with new `the_newest_version_with_a_share_beside_a_live_one_is_the_canary`, `the_overview_joins_runs_to_rollout_versions`, `tests/training.rs` (a linked device over the in-memory Weirdnet stack, an unlinked one refused) and `dates_are_utc`. Not run: the Xvfb desktop scenario (step 60 is written, not played) and a check against a real Ymir 0.16.0 node.

## 2026-11-25 21:56 EST: the upload dialog in the Flutter app (branch `feature/flutter-upload`)

Michael asked at 9:33 PM ET for the Flutter upload dialog on par with the desktop one (rollout and canary share, license, keep in `node.toml`), as far as a phone can sensibly upload, showing the `ymirctl` command where it can't hold the file.

**What changed**
- [crates/bragi-core/src/models/upload.rs](crates/bragi-core/src/models/upload.rs): `UploadSpec::ymirctl_command(file, label)`: the same upload as `ymirctl models upload FILE --node A[,B --stage A] [--name] [--adapter-for] [--license] [--no-persist] [--rollout NAME --inference NODE --canary PCT]`, quoted for a shell (a leading `~/` stays outside the quotes).
- [crates/bragi-bridge/src/api/bragi.rs](crates/bragi-bridge/src/api/bragi.rs): `UploadPreviewView`, `UploadSpecView`, `UploadedResultView`, `UploadCopyView`, `PendingUploadView`; `models_inspect_upload`, `models_upload`, `upload_command`, `models_pending_uploads`, `models_forget_upload`, `models_inference_nodes`, `upload_percent_presets`; the `uploaded` event carries the result; `StoredModelView.base`; `demo_write_gguf` for tests. `flutter_rust_bridge_codegen generate` was re-run.
- [app/lib/src/upload_ui.dart](app/lib/src/upload_ui.dart): the Upload screen (file path or Choose…, preview, node, base for an adapter, Also copy to, name, license, Keep it, rollout name, inference node, presets and a typed percentage with `parsePercent`, progress with Cancel, Resume, the result with copies, adapter note and rollout lines, and From a computer with the `ymirctl` command and Copy). The Models screen has Upload a model and Unfinished uploads (Resume, Forget).

**Checks:** `cargo fmt --check`; clippy `--workspace --all-targets -D warnings`; `cargo test --workspace --locked` 313 passed, 0 failed, 4 ignored; new `an_upload_reads_as_the_ymirctl_command_for_a_computer_that_holds_the_file`. `flutter test --dart-define=BRAGI_DEMO=true` against the regenerated bridge: the models test now uploads to the demo nodes (the no-space refusal from the second node, then stored on the first, added to the rollout, the command shown) and checks `parsePercent`; the whole Flutter suite passed (10 tests). `flutter analyze` reports only the six existing `curly_braces` infos in other files.

**Not tested:** on a real phone (the file picker copies a picked file into the app's cache on Android and iOS, which is why large files point to the command); against a real Ymir node.

**Next:** merge after CI; release 0.12.0.
## 2026-11-25 17:25 EST: v0.11.0 released

`release: v0.11.0` (`9030973`, tag `v0.11.0`), cut on main `5a6805a` after Pages and Platforms (macOS and Windows desktop) were green: the model picker tags uploaded and trained models with their origin and license from the pool catalog (Ymir's U8; needs Ymir 0.15.3), weirdnet-agent 0.5.4 and weirdnet-backends 0.4.5, and the Windows sync-test race fix. The cut's checks passed. Every platform built in the Release workflow ([run 45703741360](https://github.com/WeirdBrains/bragi/actions/runs/45703741360)); the 41 files were scanned unpacked: no host names, tailnet names, private IPs or home paths. [GitHub release](https://github.com/WeirdBrains/bragi/releases/tag/v0.11.0) with 42 files and `SHA256SUMS` (`assets` ok), plus detached `.sig` files for the six Arch packages, signed with the WeirdBrains packages key (each "Good signature", sums matching): 48 files. `bragi-bin` 0.11.0-1 on TAUR; `packaging/nix/release.json` and the site's `latestVersion` at 0.11.0 (branch `release/0.11.0-packaging`).

## 2026-11-25 15:02 EST: origin and license tags in the model picker (branch `feature/catalog-origin`)

Ymir's model-upload U8, now that Weirdnet request D shipped. `ModelInfo` has `origin` and `license` from catalog/1 (`CatalogModel::origin`/`license`, weirdnet-agent 0.5.4), taken from the first inference node that says them; `ModelInfo::origin_tag()` gives `upload, MIT` or `train` and nothing for Hub or unknown models, as the Models screen already does for a node's stored files. The desktop picker chip and the terminal model line add the tag. weirdnet-agent 0.5.4 and weirdnet-backends 0.4.5. Older inference nodes send neither field and nothing is shown; saved views without the fields still read. The Flutter app's `ModelView` is unchanged (it would need the bridge regenerated); a later slice.

**Checks:** fmt; clippy `-D warnings`; `cargo test --workspace --locked` 312 passed, 0 failed; new `the_catalog_says_where_a_model_came_from`. Not run: the Xvfb desktop scenario (the demo nodes carry no origins, so it would show nothing new) and a check against a real Ymir node with U8.

## 2026-11-25 08:25 EST: v0.10.0 released

`release: v0.10.0` (`a37f054`, tag `v0.10.0`), cut on main `012e3f2` after Pages was green: the upload dialog's rollout share is a percentage (0 to 100, presets; needs Ymir 0.15.0 on the inference node), and a node that can't copy an upload gets it from this device. The cut's checks passed. Every platform built in the Release workflow ([run 45618226541](https://github.com/WeirdBrains/bragi/actions/runs/45618226541)); the 41 files were scanned unpacked: no host names, tailnet names, private IPs or home paths. [GitHub release](https://github.com/WeirdBrains/bragi/releases/tag/v0.10.0) with 42 files and `SHA256SUMS` (`assets` ok), plus detached `.sig` files for the six Arch packages, signed with the WeirdBrains packages key (each "Good signature", sums matching): 48 files. `bragi-bin` 0.10.0-1 on TAUR; `packaging/nix/release.json` and the site's `latestVersion` at 0.10.0 (this branch).

## 2026-11-25 07:38 EST: the upload dialog's rollout share is a percentage (branch `feature/canary-percent`)

Michael asked at 7:24 AM ET for a canary percentage such as 20 or 50 (Ymir's model-upload decision 8). The 0.10.0 cut that was running then was stopped before it committed, tagged or pushed (its version edits were reverted), so the release includes this.

**What changed**
- [crates/bragi-core/src/models/ymir_rollouts.rs](crates/bragi-core/src/models/ymir_rollouts.rs) (TEMPORARY mirror): `Change::Share`, `MAX_PERCENT` (100) and `PRESETS`; `add_version` adds at weight 0, then sends the share (an older Ymir says it needs 0.15.0 and the version stays at 0); the stand-in serves shares.
- `RolloutAsk.percent` (serde alias `weight`, so unfinished uploads from 0.8.0 and 0.9.0 still read).
- Desktop: presets 0, 1, 5, 10, 20, 50 and 100% (`upload-percent-{p}`) and a typed percentage field (`parse_percent`: empty is 0, `%` allowed, over 100 refused with a message); scenario step 53 picks 20%.
- CHANGELOG; ROADMAP, roadmap.toml and the site roadmap text.

**Checks:** `cargo fmt --check`; clippy `--workspace --all-targets -D warnings`; `cargo test --workspace --locked` 311 passed, 0 failed, 4 ignored; the desktop scenario on Xvfb with the demo profile passed (screenshots 53 and 54 show the share row and the added version).

**Not tested:** against a real Ymir 0.15.0 inference node (the stand-in mirrors its split); the Flutter app (no upload dialog there yet).

**Next:** merge; release 0.10.0 after Ymir 0.15.0.

## 2026-11-25 05:58 EST: a node that can't copy an upload gets it from this device (branch `feature/upload-copy-fallback`)

The fallback `ymirctl models upload --node all` has (Ymir's U6): in `ModelsManager::copy_to` ([crates/bragi-core/src/models/upload.rs](crates/bragi-core/src/models/upload.rs)), a copy that fails for any reason other than a refusal or a cancel (the node can't reach the one the file went to, or runs an older Ymir) is sent from this device with `ModelsClient::upload` and the same local file, then saved as usual. Its result line says "X could not copy it to Y, so it was sent from this device". Refusals (no space, a name in use) are reported as before. CHANGELOG updated.

**Checks:** `cargo fmt --check`; clippy `--workspace --all-targets -D warnings`; `cargo test --workspace --locked` 310 passed, 0 failed, 4 ignored (`an_upload_is_copied_on_to_other_nodes` now checks that the node without a peer dialer gets the file from this device).

**Not tested:** against real nodes on different machines; the desktop scenario (the demo nodes still show "left out" for the full node; the result text is the core's).

**Next:** merge; release 0.10.0 with Ymir 0.15.0 (model upload U7).

## 2026-11-25 05:26 EST: v0.9.0 released

`release: v0.9.0` (`1db8f04`, tag `v0.9.0`), cut on main `77c7a43` after Pages was green: an upload can be copied on to other model nodes (`feature/upload-copies`; needs Ymir 0.14.0, released alongside). The cut's checks passed. Every platform built in the Release workflow ([run 45590082203](https://github.com/WeirdBrains/bragi/actions/runs/45590082203)); the 41 files were scanned unpacked: no host names, tailnet names, private IPs or home paths. [GitHub release](https://github.com/WeirdBrains/bragi/releases/tag/v0.9.0) with 42 files and `SHA256SUMS` (`assets` ok), plus detached `.sig` files for the six Arch packages, signed with the WeirdBrains packages key (each "Good signature", sums matching): 48 files. `bragi-bin` 0.9.0-1 on TAUR; `packaging/nix/release.json` and the site's `latestVersion` at 0.9.0 (this branch).

## 2026-11-25 04:48 EST: copy an upload on to other nodes (branch `feature/upload-copies`)

Ymir's model-upload U6 (pools) on the desktop: the file goes once to the picked node and the others copy it from there, on Weirdnet's `ModelsClient::replicate` (weirdnet-backends 0.4.4); needs Ymir 0.14.0 on the nodes.

**Changed:**
- bragi-core [`models/upload.rs`](crates/bragi-core/src/models/upload.rs): `UploadSpec.copy_to`, `UploadedView.copies`, `ModelsManager::copy_to` (each node's free space less what it keeps first, then `replicate`, then each copy recorded with `/ymir/uploads/1`). `demo_models::service_peering` gives a test or demo service a peer dialer.
- Desktop [`view/upload.rs`](crates/bragi-desktop/src/view/upload.rs): "Also copy to" chips for the other ready nodes and one result line per node.

**Checks:** `cargo fmt`; clippy `--workspace --all-targets -D warnings`; `cargo test --workspace` 310 passed, 0 failed, 4 ignored (new: `an_upload_is_copied_on_to_other_nodes`: copied and saved, a full node left out, a node without a peer dialer refused). The desktop scenario passed under Xvfb with the demo profile, with new shots 55-56 (the copy chips; the second demo node, which keeps more free than it has, left out). No Dart changed, so Flutter was not run.

**Not tested:** a successful copy in the desktop demo (the demo nodes have no peer dialer, and the second is full); against real Ymir nodes. Bragi does not fall back to sending from this device when a node can't copy (ymirctl does).

**Next:** release with Ymir 0.14.0.

## 2026-11-25 04:18 EST: v0.8.0 released

`release: v0.8.0` (`32a517d`, tag `v0.8.0`), cut on main `60ec613` after Pages was green: an upload can join a rollout from the desktop dialog (`feature/upload-rollout`, merged in `60ec613`; needs Ymir 0.13.0, released alongside). The cut's checks passed. Every platform built in the Release workflow ([run 45576032646](https://github.com/WeirdBrains/bragi/actions/runs/45576032646)); the 41 files were scanned unpacked: no host names, tailnet names, private IPs or home paths. [GitHub release](https://github.com/WeirdBrains/bragi/releases/tag/v0.8.0) with 42 files and `SHA256SUMS` (`assets` ok, marked latest), plus detached `.sig` files for the six Arch packages, signed with the WeirdBrains packages key (each "Good signature", sums matching): 48 files. `bragi-bin` 0.8.0-1 on TAUR; `packaging/nix/release.json` and the site's `latestVersion` at 0.8.0 (this branch). The first `github 0.8.0` attempt was interrupted before it started and was run once.

## 2026-11-25 03:14 EST: add an upload to a rollout (branch `feature/upload-rollout`)

The follow-up to the upload dialog, on Ymir's new `/ymir/rollouts/1` (Ymir 0.13.0). The rollout decisions were made by llm dev from Michael's upload answers (any linked device, narrowed by the node's `rollout_callers`; above 10% needs scored eval rows or force, so Bragi offers 0-10% only; changes kept in `node.toml` by default). Michael can override them.

**Changed:**
- bragi-core [`models/ymir_rollouts.rs`](crates/bragi-core/src/models/ymir_rollouts.rs): a small, temporary copy of Ymir's rollouts wire types (list and add only), `add_version` (lists the rollout, adds the next version with the weight capped at 10) and the demo/test stand-in. `UploadSpec.rollout` (`RolloutAsk`: inference node, rollout name, weight) and `UploadedView.rollout`; the `ymirctl` rollout commands now show only when the rollout was not added, with `--inference`.
- The demo nodes answer rollouts in memory.
- Desktop [`view/upload.rs`](crates/bragi-desktop/src/view/upload.rs): the dialog's rollout section (name, inference node, weight 0/1/5/10, the canary note) and the result line or the refusal.

**Checks:** `cargo fmt`; clippy `--workspace --all-targets -D warnings`; `cargo test --workspace` 309 passed, 0 failed, 4 ignored (new: `an_upload_joins_a_rollout_as_its_next_version`). The desktop scenario passed under Xvfb with the demo profile, with new shots 53-54 (the rollout section and the added version). No Dart changed, so Flutter was not run.

**Not tested:** against a real Ymir inference node over the network (Ymir's own testkit covers `/ymir/rollouts/1` between three daemons).

**Next:** release; the Flutter dialog; drop the temporary wire copies when Weirdnet has the hooks.

## 2026-11-25 02:48 EST: v0.7.0 released

`release: v0.7.0` (`420b783`, tag `v0.7.0`): the desktop upload dialog (Ymir's model-upload U4, `feature/model-upload-dialog`, merged in `6cb6742`). Before the cut, `fix/handoff-agents-link` (`1fdaf57`) made the Pages link check pass again. The Release workflow's `check` job first failed on a `bragi-serve` port race in `no_binary_is_reported`; a rerun of that job passed, and `fix/serve-test-port-race` (`17c58b9`) has the test try a fresh port. Every platform built ([run 45566365976](https://github.com/WeirdBrains/bragi/actions/runs/45566365976)). Artifacts scanned (archives and packages unpacked): only the RPM packager field (the public maintainer address) matched; no paths or host names. [GitHub release](https://github.com/WeirdBrains/bragi/releases/tag/v0.7.0) with 42 files and `SHA256SUMS` (`assets` ok), plus detached `.sig` files for the six Arch packages, signed with the WeirdBrains packages key (each "Good signature", sums matching). `bragi-bin` 0.7.0-1 on TAUR; `packaging/nix/release.json` and the site's `latestVersion` at 0.7.0.

## 2026-11-25 02:07 EST: model upload dialog on the desktop (branch `feature/model-upload-dialog`)

Ymir's model-upload plan, U4, desktop (GPUI) first, on Ymir 0.12.0's upload. Michael's calls: any linked device may upload, the license is optional, the node keeps max(10%, 20 GiB) free, an upload is kept in the node's `node.toml` by default, and rollouts are not set from Bragi yet (the result shows the `ymirctl` commands; `/ymir/rollouts/1` is the follow-up).

**Changed:**
- `weirdnet-agent` 0.5.2, `weirdnet-backends` 0.4.4, `weirdnet-model` 0.5.3. `StoredView` carries `base`, `origin` and `license`, and lists adapters too.
- bragi-core [`models/upload.rs`](crates/bragi-core/src/models/upload.rs): `inspect` (what the file is, hashed here), `UploadSpec`, `ModelsManager::upload` over models/1 (progress that never goes back; `exists` with the same revision counts as already there; the llama.cpp next-start note for an adapter), refusals explained (no-space names `upload_keep_free`), and pending uploads in the pool's `uploads.json` for Resume. `Core::models_inspect_upload`, `models_upload`, `models_pending_uploads`, `models_forget_upload`.
- [`models/ymir_uploads.rs`](crates/bragi-core/src/models/ymir_uploads.rs): a small, temporary copy of Ymir's `/ymir/uploads/1` wire types, which records the upload with the node. It goes away when Weirdnet's "upload placed" hook on `ModelsService` lands (asked of the Weirdnet Lead).
- The demo nodes take uploads; the second keeps more free than its disk has, so it refuses with no-space.
- Desktop [`view/upload.rs`](crates/bragi-desktop/src/view/upload.rs): Upload… on the Models screen opens the dialog (file picker, preview, node, adapter base, name, license, keep in `node.toml`, progress with Cancel, the result with the rollout commands), and the screen lists unfinished uploads with Resume and Forget. The bridge passes the `uploaded` event on; the Flutter dialog comes later.
- `scripts/desktop-scenario.sh` retries its cleanup while a document portal still holds `run/doc`.

**Checks:** `cargo fmt`; clippy `--workspace --all-targets -D warnings`; `cargo test --workspace` 307 passed, 0 failed, 4 ignored (new: 5 upload tests in bragi-core, 2 in the desktop). The desktop scenario passed under Xvfb with the demo profile (`scripts/desktop-scenario.sh`), with new shots 47-52: preview, progress, unfinished, resumed, no-space, adapter. No Dart changed, so Flutter was not run.

**Not tested:** against a real Ymir node over the network; the llama.cpp restart note end to end (the demo's engine takes adapters live, so the scenario's shot 52 seeds that note; the text comes from bragi-core); the system file picker (the scenario sets the path).

**Next:** merge, release, then `/ymir/rollouts/1` in Ymir and a rollout section in the dialog; the Flutter dialog.

## 2026-11-24 13:25 EST: cert-only default for Ymir server links (in progress, branch `feat/cert-only-default`)

Michael approved: linking a Ymir server is cert-only by default, with an explicit opt-out for the full-identity link.

**Done on the branch (0a5768f, WIP, pushed):**
- bragi-core: `is_server_link(code)` (a `weirdnet-ymir://link/` code from `ymirctl pool link`). `pair_accept` remembers it, and `pair_accept_confirm` then links cert-only as an inference node (certificate plus service grant, no seed). This covers every front end (desktop, mobile bridge, MCP, serve) with no bridge type change. `pair_accept_confirm_full` is the opt-out; `pairing_is_server()` is exported.
- bragi-tui: `link CODE` follows the default; `--server model|inference` picks the role; new `--full-identity` sends the seed (it conflicts with `--server`). A failed cert-only link tells you a Ymir older than 0.8.0 can't take one (update it, or pass `--full-identity`). Help text updated. Parser tests cover `--full-identity` and the conflicts.
- `tests/scenario.rs` `a_ymir_server_is_linked_cert_only_without_the_seed`: adds the default confirm (expects an inference grant, no seed) and `pair_accept_confirm_full` (expects the seed). **Passed** on the box.

**Left:**
1. Run the bragi-tui cli tests (`cargo test -p bragi-tui cli`; interrupted before finishing), the full suite and clippy.
2. Front ends that show a link code dialog (desktop `view.rs`, Flutter via the bridge) could show "cert-only" for a server code and offer the full-identity opt-out. The core default already applies to them.
3. Docs: README, site `docs_started.dart`, CHANGELOG (Changed: server codes are cert-only by default), DESIGN in Ymir (B-section), roadmap.
4. `scripts/ymir-link-e2e.sh`: the default should now be cert-only (`BRAGI_E2E_CERT_ONLY` becomes the default; add a full-identity case).
5. Vendor weirdnet-project-helpers 0.3.3 (`python3 ../weirdnet-project-helpers/vendor.py .`, `helpers = "0.3.3"`), then release per [AGENTS.md](AGENTS.md): `weirdnet_release.py --dry-run`, cut, Release workflow, dist scan, `assets`, packaging, `taur` (the signing machine has `taurctl`), site and roadmap.

**Other branches pushed so nothing stays on one machine:** `feat/training-consent` (merged earlier, kept).

**Local state:** a local clone on the build machine has a stash "local: bragi-serve ensure.rs" (the older `crates/bragi-serve/src/ensure.rs` change, kept, not applied).

## 2026-11-24: v0.6.0 released

`release: v0.6.0` (`30f294e`, tag `v0.6.0`): Ymir rollouts in the model list (`feat/rollout-versions`, merged in `c47608f`). The Release workflow passed on every platform ([run 45431928019](https://github.com/WeirdBrains/bragi/actions/runs/45431928019)). Artifacts scanned: only the RPM packager field (the public maintainer address) matched; no paths or host names. [GitHub release](https://github.com/WeirdBrains/bragi/releases/tag/v0.6.0) with 42 files and `SHA256SUMS`, `assets` ok; `bragi-bin` 0.6.0-1 on TAUR (`46ace46`); `packaging/nix/release.json` and the site's `latestVersion` at 0.6.0.

## 2026-11-24: Ymir rollouts as one row (branch `feat/rollout-versions`)

Ymir's training plan P4b (Ymir main `a48fed7`) lists a rollout's name and each `name@N` in the catalog. [`catalog`](crates/bragi-core/src/catalog.rs): `ModelInfo::versions` holds a rollout's versions under its name's row (only when the name is listed too, and only for a numeric `@N`). `CatalogView::model(id)` finds a row or a folded version, so `nodes_for`, `context_for`, `refusal` and `Core::pick_model` still work for a pinned version, and the desktop keeps a pinned choice. The terminal picker lists versions after their name. The Flutter and GPUI pickers show the name only. Docs: [CHANGELOG.md](CHANGELOG.md), [roadmap.toml](roadmap.toml) (B2; [ROADMAP.md](ROADMAP.md) and the site regenerated), and the B2 line in Ymir's [DESIGN.md](https://github.com/WeirdBrains/ymir/blob/main/docs/DESIGN.md#bragi-milestones) (`6099104`).

Checks: `cargo fmt`; clippy `--workspace --all-targets -D warnings`; `cargo test --workspace` 300 passed, 0 failed, 4 ignored, including `catalog::rollout_versions_fold_under_their_name`.

Not tested: against a real Ymir rollout over Weirdnet; a version picker in the Flutter and GPUI shells (not built).

## 2026-11-24: v0.5.0 released

`release: v0.5.0` (`bd6e41e`, tag `v0.5.0`) with cert-only links for Ymir servers (`bragi-tui link CODE --server inference|model`, Ymir's M10) and `weirdnet-sync` 0.7.1 / `weirdnet-device-link` 0.4.2. The feature work is on `feat/cert-only-servers`, merged in `b3e130b`. The Release workflow passed on every platform ([run 45412760862](https://github.com/WeirdBrains/bragi/actions/runs/45412760862)). Artifacts scanned: the only match was the RPM packager field (the public maintainer address); no paths or host names. [GitHub release](https://github.com/WeirdBrains/bragi/releases/tag/v0.5.0) with 42 files and `SHA256SUMS`, `assets` ok; `bragi-bin` 0.5.0-1 on TAUR (`ac101f5`); `packaging/nix/release.json` and the site's `latestVersion` at 0.5.0.

Not decided: whether Bragi should link a Ymir server cert-only by default once a Ymir with M10 is released (now opt-in with `--server`).

## 2026-11-24: v0.4.0 released

`release: v0.4.0` (`268ff23`, tag `v0.4.0`) with a collapsible history sidebar (the choice is remembered) and switching between more than one Ymir pool. Feature work is on `feat/history-bar-and-pools`, merged in `b3bdcf4`. The Release workflow passed on every platform ([run 45405900042](https://github.com/WeirdBrains/bragi/actions/runs/45405900042)); artifacts scanned (only the CI cargo-registry paths, as in 0.3.0; no host names); [GitHub release](https://github.com/WeirdBrains/bragi/releases/tag/v0.4.0) with 42 files and `SHA256SUMS`, `assets` ok; `bragi-bin` 0.4.0-1 on TAUR; `packaging/nix/release.json` and the site's `latestVersion` at 0.4.0.

## 2026-11-24: collapsible history sidebar and more than one Ymir pool (branch `feat/history-bar-and-pools`)

Michael's pt169u (alternate Ymir and Bragi phases; make Bragi's history bar collapsible; switch which Ymir pool the device is linked to). Core [`pools`](crates/bragi-core/src/pools.rs): more than one saved identity (the first stays at the profile root; later ones under `pools/<id>/`); `add_pool`, `switch_pool`, `rename_pool`, `remove_pool`; a switch goes offline, changes the active pool, and reloads history and node addresses. Pairing a new pool uses the existing link-code offer. [`appearance`](crates/bragi-core/src/appearance.rs): `Preference.sidebar_collapsed`, remembered. History can `switch_root`. Bridge, Flutter, GPUI and the terminal app all list, add, switch, rename and forget pools, and can hide the history bar (F10 / Ctrl-B / `/sidebar` in the terminal; History in the desktop title bar; Hide on a wide Flutter layout; a phone keeps the drawer). CLI: `bragi pool list|add|switch|rename|remove`. Docs: [CHANGELOG.md](CHANGELOG.md), [roadmap.toml](roadmap.toml) (v0.4; [ROADMAP.md](ROADMAP.md) and the site regenerated).

Checks: `cargo fmt --all --check`; clippy `--locked --workspace --all-targets -D warnings`; `cargo test --locked --workspace` (0 failed, including new `pools` tests); `flutter test --dart-define=BRAGI_DEMO=true` with the bridge library: settings, scenario, code, theme and models all passed (the scenario now scrolls Settings so Clear all history is tappable on a phone).

Not tested: a real desktop window, the phone app on a device, or switching real Ymir pools (core and the terminal app were tried against demo/memory nodes; Flutter against the in-process demo).

Next: after v0.4.0, the unreleased cert-only server link work already on main.

## 2026-11-24: cert-only Ymir servers (branch `feat/cert-only-servers`)

Michael's pt169u (alternate Ymir and Bragi phases); the Bragi side of Ymir M10, which is on Ymir main (`cf5c1ba`, not yet released). Workspace: `weirdnet-sync` 0.7.1 (service devices) and `weirdnet-device-link` 0.4.2 (`Pairing::send_credential`), `zeroize`. Core [`identity`](crates/bragi-core/src/identity.rs): the seed is optional in sync 0.7, `user_keypair()`, `service_bundle(peer, roles, types)` (zeroized, saved), `is_service(peer)`. [`core`](crates/bragi-core/src/core.rs): `ServerRole { Inference, Model }` (Ymir's role and type names: inference gets `ymir.pool` and `ymir.context`, model none); `pair_accept_confirm_server(matches, role)` grants the server a scoped certificate and sends only the user key and the bundle, never the seed. A Ymir from before M10 (device-link 0.3.2) can't take a credential and the error says to update it and link again; there is no fallback that sends the seed. Terminal: `bragi-tui link CODE --server inference|model` (a code, not an address). Docs: README, the site's getting-started page, [roadmap.toml](roadmap.toml) (B1; [ROADMAP.md](ROADMAP.md) and the site regenerated), CHANGELOG.

Cert-only is opt-in. Making it the default for Ymir codes would break linking with released Ymir 0.7.0, so that is left for Michael once a Ymir with M10 is out.

Checks: `cargo fmt`; clippy `--workspace --all-targets -D warnings`; `cargo test --workspace` on the merge with the pool switching: 299 passed, 0 failed, 4 ignored (new: `a_ymir_server_is_linked_cert_only_without_the_seed`); the real cross-app e2e [`scripts/ymir-link-e2e.sh`](scripts/ymir-link-e2e.sh) against Ymir main's `ymird`/`ymirctl` passed both ways, full and with `BRAGI_E2E_CERT_ONLY=1` (node2 `pool link --cert-only`, Bragi sends a credential; node2 has `credential.json` and no `seed`, and node1's roster lists it). A first cert-only run where the env var didn't reach Bragi also showed Ymir's refusal: Bragi sent the seed, `ymirctl` said nothing was stored. No Flutter or bridge change, so the Flutter checks were not rerun.

Not tested: real machines; the Flutter and GPUI apps have no cert-only option yet (terminal and core only).

## 2026-11-24: v0.3.0 released

`release: v0.3.0` (`4595f13`, tag `v0.3.0`) with the training consent; the Release workflow passed on every platform; artifacts scanned (only the CI cargo-registry paths and the rpm packager line, as in 0.2.1); [GitHub release](https://github.com/WeirdBrains/bragi/releases/tag/v0.3.0) with 42 files and `SHA256SUMS`, `assets` ok; `bragi-bin` 0.3.0-1 on TAUR; `packaging/nix/release.json` and the site's `latestVersion` at 0.3.0.

## 2026-11-24: training consent per chat (branch `feat/training-consent`)

Michael's pt169u (alternate Ymir and Bragi phases for full support); the Bragi side of Ymir's training plan P2, whose Ymir side (the export) is on Ymir main. Core [`context`](crates/bragi-core/src/context.rs): `Train { on, at_ms }` in Ymir's `{conversation}/train` record (its own register, so a `meta` rewrite never clears it), `set_train`, read into `StoredConversation::train`; [`history`](crates/bragi-core/src/history.rs): `Conversation::training`, `History::set_training`; [`core`](crates/bragi-core/src/core.rs): `Core::set_training`. Bridge: `ConversationView.training`, `set_training(id, marked)` (regenerated with frb 2.13.0). Flutter: [`training_ui.dart`](app/lib/src/training_ui.dart), a cap button in the chat's app bar that asks before marking (no tooltip: its long-press overlay broke the message menu's delete dialog in the scenario). GPUI: a Training button next to Agent, with a notice. Terminal: `/train`, `/train off`, `· training` in the chat's title, help line. Also the bridge's `set_theme_preference` updates the two fields it sets (clippy `needless_update`). Docs: [CHANGELOG.md](CHANGELOG.md), [roadmap.toml](roadmap.toml) (B4; [ROADMAP.md](ROADMAP.md) and the site regenerated), the site's features page.

Checks: `cargo fmt`; clippy `--workspace --all-targets -D warnings`; `cargo test --workspace`: 292 passed, 0 failed, 4 ignored (new: `history::training_consent_is_off_until_set_survives_a_save_and_is_ymirs_record`; the scripted terminal test marks and withdraws); `flutter analyze`; `flutter test` with the bridge library: 10 passed (the scenario marks, confirms and withdraws); the desktop scenario under Xvfb ran through its new training step to the end (no screenshots: ImageMagick's `import` is not on this box).

Not tested: against a real Ymir node's export (Ymir's own end-to-end test covers the export from synced records); Android and iOS builds.

Next: Bragi B4 push wake-ups need `weirdnet-push` (unpublished); B3 store listings and signing need Michael's accounts and keys.

## 2026-11-23: the desktop link and code mode on the phone (branch `feature/desktop-link`)

Michael's pt143u–pt147u. Core [`link`](crates/bragi-core/src/link/mod.rs): a `bragi-link` app over weirdnet-net 0.8.2 app calls (Noise, peer-ID authenticated), served by `LinkServer` on LAN/VPN addresses (port 47480) and through the Weirdnet relay, with `LinkClient` on the phone (routes Auto, Direct, Weirdnet). Pairing by QR URL (`weirdnet-bragi://desktop?…`: peer ID, a 128-bit one-use secret, addresses, relay) or a 28-character typed code (IPv4, port, 48-bit secret, 40-bit fingerprint of the peer ID); five minutes, one use, five wrong tries. A 256-bit token is stored hashed on the desktop; revocation and settings reload on change. Paths are confined to the shared roots (`confine.rs`). Shell runs stream NDJSON frames with a keepalive, so a dropped call kills the process group; optional confirm on the desktop. `Project`/`CodeSession` run code mode the same way locally or on a desktop. `bragi linker` (terminal app) pairs, serves, lists, revokes and manages roots. No Weirdnet crate release was needed. Protocol: [docs/desktop-link.md](docs/desktop-link.md).

Bridge [`api/code.rs`](crates/bragi-bridge/src/api/code.rs) (regenerated with frb 2.13.0): link, project, context, ask/answered, review, apply/undo, shell stream and stop, prompt history and notifications. App: `CodeApi`/`CodeController`, `code_ui.dart` (mode toggle, project bar and picker, context browser, review card, shell panel, bell), `desktop_pair_screen.dart` (mobile_scanner, typed code, route, unpair), Up/Down history in the message box; camera permission on Android and iOS.

Tests: core link tests on the memory transport (pairing, read/write/undo, confinement, shell stream and stop, one-use and expired codes, revoked and stolen tokens, confirm, routes) plus an ignored live test (direct, relay-only and typed-code pairing) that passed on a real network; `bragi-tui` linker CLI tests; Flutter `code_test.dart` (bridge: toggle, project, context, shell, a code ask saved in the conversation, history, bell; stand-in: hunk review and apply, QR and typed pairing, route, unpair). `theme_test` and `scenario_test` also failed on `main` (984de07); fixed here: the theme editor previewed during a build and its Saved message was scrolled away, and the scenario's oldest conversation was below the drawer's fold. Checks: `cargo fmt --check`, workspace clippy `-D warnings`, `cargo test` on `bragi-core`, `bragi-tui`, `bragi-bridge`, `flutter analyze`, `flutter test`.

Not done: the GPUI desktop app has no linker screen (use `bragi linker`) and no code mode of its own; the typed code carries IPv4 only (the QR carries IPv6 and the relay too). Release 0.2.0 is held for the workspace version-pin decision.

## 2026-11-23: chat and code as modes of one conversation (branch `feature/chat-code-modes`)

Michael's t141u/t142u. Core: `history::Mode { Chat, Code }` on `ChatMessage.mode` (serde default chat; stored as `ext.mode` only when code, so old records and other apps read as chat); `ChatMessage::new`; `Core::record(conversation, messages)` saves turns made outside a chat request (new conversation when `None`); `merge_turns` joins consecutive same-role messages (used by `agent_messages`, so a recorded shell run after a question still alternates). Bridge `MessageView.mode` (regenerated with frb 2.13.0); desktop and phone show "code mode" in a code turn's meta line.

Terminal app: `View::Chat`/`View::Code` are now the two modes of the same conversation (`App::mode()`, `App::set_mode()`); both draw `ui::draw_transcript`; code mode puts the Context panel where the sidebar is, and the diff review replaces the transcript only while reviewing. Code requests send the conversation (newest turns within a third of the prompt budget, `code::conversation_turns`, reserved out of the window before fitting files). Finished, stopped and failed code turns are recorded; shell runs are recorded as a turn of the current mode (folded to their last 30 lines on screen). Tab/Shift-Tab switch modes (there is no completion popup to defer to); Ctrl-B moves focus to the sidebar. Review keeps Tab as next hunk.

Not done: the desktop and phone apps have no code mode (no project, files or shell there), so they show code turns but can't make them. Flutter `theme_test` and `scenario_test` fail locally with a setState-during-build in `AppearanceController.previewPalette` and a scenario error; this branch only added a field and a label there, and I didn't check them on `main`.

## 2026-11-23: prompt history, notifications, shell, scaffolding, folders, clickable lists, kept Thinking, a code transcript (branch `feature/project-scaffold-shell`)

Michael's t134u–t140u. New in `bragi-core`: `project` (scaffold a missing or empty folder: a starter readme and `.gitignore`, never overwrite), `shell` (run `!cmd` under `sh -c` with a time limit, stop flag and 64 KiB tail per stream; only user-typed lines reach it), `prompt_history` (JSONL per profile, global or per project by path hash, cap 1000), `notifications` (JSONL log, cap 200, unread count). `attach::from_path` takes a folder. `agent::pump` now keeps reasoning and its timing with the saved answer.

Terminal app: Up/Down history (first/last line only); F9 / `/notifications` / header bell; toasts pause while hovered; `!cmd` in chat and code; `/root` scaffolds; picker filter matches folders; theme, file and notice rows clickable with wheel; code mode keeps a transcript (`CodeState::log`: asks, answers with Thinking, shell runs, notes) rendered with `bottom()` so it scrolls; Thinking folds per message (click) with Ctrl-K resetting all.

Not done in this branch: the desktop and phone apps have no code mode, so t140 is terminal-only; prompt history, the notification list and clickable Thinking per message on desktop/phone are still to do (folder attach works there through `from_path`). Tests: `code::tests` (transcript, new root), `tests/mouse.rs::prompt_history_notifications_lists_and_the_shell`, core unit tests for each module.

## 2026-11-22: code mode fits the model's context window (branch `fix/code-context-budget`)

**What changed**
- Bug: code mode only capped its context in bytes (512 KiB), never in tokens, so 22 files (215 KiB of Python) went out as a ~92k-token prompt to a model served with a 32k window, and the server's raw `exceed_context_size_error` JSON was shown.
- [`budget.rs`](crates/bragi-core/src/code/budget.rs): a high-side token estimate (about 2.5 bytes per token on the built prompt), a `TokenBudget` per window (a quarter, 1k to 8k, kept for the answer, plus a 5% margin), and `fit`, which leaves out files in order until the prompt fits. A rewrite's target is never left out; if it alone is too big, the request is refused with the numbers. `Overflow` reads llama.cpp, vLLM and OpenAI context-size refusals.
- [`catalog.rs`](crates/bragi-core/src/catalog.rs): `min_context` and `CatalogView::context_for`, the smallest window among the nodes serving a model (the picker still shows the largest).
- Terminal app ([`code.rs`](crates/bragi-tui/src/code.rs), [`app.rs`](crates/bragi-tui/src/app.rs), [`ui.rs`](crates/bragi-tui/src/ui.rs)): requests are fitted to the catalog's window before they go out, with a note naming the files left out. The context panel and the file picker show the estimate against the model's window. On a context-size refusal it learns the server's window and token ratio, asks again once with fewer files, and otherwise shows a plain message.
- MCP code tools ([`code.rs`](crates/bragi-mcp/src/code.rs)): the same fit when the catalog lists a window (files left out are listed under `skipped`), and a plain message for a refusal.

**Tests**: `budget` unit tests (estimate, budget, the reported 22-file case, rewrite target, nothing fitting, ratio, refusal parsing), `catalog` (smallest window), and terminal-app tests for fitting and the one retry. `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings` and `cargo test` on `bragi-core`, `bragi-tui` and `bragi-mcp` (216 passed; scoped, with debug info off and `-j 2`, because a full workspace test link saturated the build disk). An earlier `cargo clippy --workspace --all-targets -- -D warnings` passed before the MCP change. No Dart changed, so `flutter analyze` and `flutter test` were not run.

**Not tested**: against a live Ymir node serving Inkling 2 Mini. Endpoints with no catalog entry have no known window until a refusal teaches it (no `/props` query yet). Chat mode doesn't trim history to the window yet.

## 2026-11-22: LF-37, outside MCP servers for chats (branch `feature/mcp-client`)

**What changed**
- [`mcp_servers.rs`](crates/bragi-core/src/mcp_servers.rs): the user's list of outside MCP servers (`mcp-servers.json`, no secrets). Secrets (env values, bearer tokens, header values) go to the sealed store or the OS keychain, like API keys. Scopes (`all`, a folder or a session), "always allow" per tool, a 16-server cap, and a warning before adding a stdio server (it runs as the user). The protocol client and tool-call loop are `weirdnet-mcp` (`client`, `agent`) and `weirdnet-agent`.
- Chat on an endpoint runs that loop with the MCP servers that apply to the chat.
- [`mcp_cli.rs`](crates/bragi-tui/src/mcp_cli.rs): `bragi mcp-servers` (and `bragi-tui mcp-servers`) can list, add, remove, enable, disable, test and revoke. Secrets are never typed on the command line: `--env NAME`, `--bearer-env VAR` and `--header NAME=VAR` read the environment at add time.

**Tests**: save/load and the CLI in the workspace; [`crates/bragi-core/tests/mcp_client.rs`](crates/bragi-core/tests/mcp_client.rs) drives the tool-call loop against a mock endpoint and a demo server.

**Not tested**: a real remote MCP server; the Settings UI for the list (the store and the CLI are in; a dedicated screen is not).

## 2026-11-22: mouse in the terminal app (branch `feature/tui-mouse`)

**What changed**
- Clicks, wheel and hover in the terminal app, via a hit map built while drawing ([`hit.rs`](crates/bragi-tui/src/hit.rs), [`app/mouse.rs`](crates/bragi-tui/src/app/mouse.rs)). Most clicks are the same key the button already has.
- `--mouse` / `--no-mouse` and `BRAGI_TUI_MOUSE`; F8 toggles reporting so text can be selected as usual. The event loop (now in [`entry.rs`](crates/bragi-tui/src/entry.rs)) turns capture on and off to match, and a panic hook turns it off so a crash doesn't leave the shell eating clicks.
- The Theme screen and the file picker keep their windows; a click outside or on the close button dismisses them when it is safe (not mid-edit).

**Tests**: `crates/bragi-tui/tests/mouse.rs` (clicks, wheel, hover, F8).

**Not tested**: click-to-select on every Theme row and every file-picker row (those screens landed after this branch; the windows still take clicks and close).

## 2026-11-22: LF-39, one `bragi` command: the terminal app is in it (branch `feature/bragi-single-binary`)

**What changed**
- [`entry.rs`](crates/bragi-tui/src/entry.rs): the terminal app's start-up moved out of `bragi-tui`'s `main` into `bragi_tui::entry::run(prog, args)`. [`main.rs`](crates/bragi-tui/src/main.rs) of `bragi-tui` is now a thin call to it (the compatibility binary).
- [`main.rs`](crates/bragi-desktop/src/main.rs) of `bragi`: `dispatch` reads the first word: `mcp`, `serve` (with `ensure` and `status`), `desktop`, `--help`/`-h`, else the terminal app, run in-process with the whole command line (none, a path, `status`, `link`, `--demo`, `--no-serve`…). A path named like a subcommand is `./serve` or `--path serve`. The old `terminal.rs` hand-off is gone.
- The window is the `gui` feature of `bragi-desktop` (default on): GPUI, cpal, qrcode and the rest are optional. `cargo build -p bragi-desktop --no-default-features` makes a `bragi` with no GUI libraries, and `bragi desktop` there says so. On Windows the exe is a console program now (the terminal app needs it) and `desktop` detaches the console.
- `find_spawn` (bragi-serve) still finds the right program: `bragi` spawns itself with `serve`; the `bragi-tui` alias, a symlink, resolves to `bragi`.
- Packaging: the desktop entry is `Exec=bragi desktop`; the macOS app starts a `bragi-app` script that runs `bragi desktop`; the Windows Start menu shortcut passes `desktop`. The Linux archives and nfpm, Arch and Nix packages link `bragi-tui` to `bragi`; the terminal-only archive and package are built with `--no-default-features`. Windows keeps a `bragi-tui.exe`. [README](README.md), the [Getting started guide](site/lib/pages/docs_started.dart) and the [CHANGELOG](CHANGELOG.md) say so; the FreeBSD job builds the no-window `bragi`.
- Tests: `dispatch` for none, path, a path named like a subcommand, `desktop`, `mcp`, `serve`, `--help`.

**Not tested**: the packaging changes (no nfpm, macOS or Windows here); the Windows console behaviour; the FreeBSD build.

## 2026-11-22: LF-35, the file picker in the terminal app's code mode (branch `feature/tui-file-picker`)

**What changed**
- [`picker.rs`](crates/bragi-tui/src/picker.rs): the picker's state. The tree comes from the `ignore` walker (`.gitignore`, global and exclude files, dotfiles) with `target`, `node_modules`, `.git`, `dist`, `build` and similar always hidden, folders first, empty folders dropped, at most 20,000 entries (it says when it cuts). Folders open and shut; a `/` filter is a fuzzy match on the path that lists the best matches first.
- [`code.rs`](crates/bragi-tui/src/code.rs): `CodeState::set_file` adds or removes one file within the existing limits (64 files, 256 KiB a file, 512 KiB in all); taking out a file that came with a folder swaps the folder for its other files. `/files` (also `/pick`, `/f`) is a new code command.
- [`app.rs`](crates/bragi-tui/src/app.rs) and [`ui.rs`](crates/bragi-tui/src/ui.rs): the picker is a window over code mode. Open it with `f` (empty message box), Ctrl-F or `/files`. Keys: arrows or j k, → l and ← h, Space, Enter, `/`, Ctrl-U, `c`, Esc. Markers show what is in the context (half for a part of a folder), the bottom line a size bar against the limit. Keys are in the status bar and the help.
- [`cli.rs`](crates/bragi-tui/src/cli.rs): `bragi PATH` with a folder starts with an empty context and the picker open; with a file it preselects that file, as before.
- The picker only lists the root and sets paths relative to it, so suggest, rewrite and hunk review stay inside it. It uses existing theme roles only; [`theme.rs`](crates/bragi-tui/src/theme.rs) is untouched.
- [README](README.md) and the [Getting started guide](site/lib/pages/docs_started.dart) describe it.

**Tests**: tree building with ignore rules, hidden folders and ordering; open, shut and cursor moves; fuzzy filtering; selecting and the context limits (a folder swapped out, too many bytes, a file too big); and [`tests/picker.rs`](crates/bragi-tui/tests/picker.rs), which drives the keys and draws on the test backend.

**Not tested**: a very large tree (the 20,000 cap is by construction); a real terminal other than a pseudo-terminal capture; mouse use (there is none).

## 2026-11-22: LF-33, theme switcher and editor in every front end (branch `feature/theme-editor`)

**What changed**
- [`crates/bragi-core/src/appearance.rs`](crates/bragi-core/src/appearance.rs): one `Appearance` API on weirdnet-themes 0.2.0 (list, preference with light/dark/system mode, palette, edit roles, duplicate, rename, delete, reset, import, export). The choice is saved in `appearance.json` in Bragi's data dir; user themes use the weirdnet TOML format, so they work across front ends and other Weirdnet apps.
- Terminal: [`themes_screen.rs`](crates/bragi-tui/src/app/themes_screen.rs), opened with F7 or Ctrl+G and listed in the header menu. Live preview, role editor with hex input. The theme module now uses the crate's `ColorDepth`, `builtin("bragi")` and roles; `BRAGI_THEME` and the old loader are gone.
- Desktop (GPUI): [`view/themes.rs`](crates/bragi-desktop/src/view/themes.rs), a Themes screen from the menu with the same actions and live preview.
- Flutter (Android/iOS): [`theme_ui.dart`](app/lib/src/theme_ui.dart), [`appearance.dart`](app/lib/src/appearance.dart), Theme entry in the drawer; bridge functions in [`api/bragi.rs`](crates/bragi-bridge/src/api/bragi.rs). `BragiColors` is now dynamic, so `const` was removed from widgets that read it.
- VS Code extension: no change; it follows the editor's own theme.

**Checks**: see the final report of this branch; widget tests in [`tests/themes.rs`](crates/bragi-tui/tests/themes.rs) and [`app/test/theme_test.dart`](app/test/theme_test.dart).

**Left**: screenshots in `~/.bragi-theme-shots/` (desktop and Flutter not yet captured); desktop has no automated UI test beyond `Theme::from_palette`.

## 2026-11-22: LF-40, the extension does what Bragi does (branch `feature/extension-parity`)

**What changed**
- [`docs/design/extension-parity.md`](docs/design/extension-parity.md): the feature-by-feature table (apps vs extension), the settings plan, the serve API additions and what is left.
- `bragi-serve` ([`manage.rs`](crates/bragi-serve/src/manage.rs)): management routes behind the token for links (list, add by address, remove, accept and confirm a link code), OpenAI-compatible endpoints (list, add, edit, delete, test, models; keys accepted, never returned) and sessions (list, delete). They call the `Core` methods the apps use. Tests: [`tests/manage.rs`](crates/bragi-serve/tests/manage.rs).
- Extension ([`editors/vscode`](editors/vscode)): `package.json` configuration is five sections (Connection, Models, Chat, MCP, Advanced) with title, description, order and scope per key (machine scope for anything that runs a program or points at a server); new commands and the Connections view ([`commands.ts`](editors/vscode/src/commands.ts), [`manage.ts`](editors/vscode/src/manage.ts), [`tree.ts`](editors/vscode/src/tree.ts), [`settings.ts`](editors/vscode/src/settings.ts)); `ServeManager.stop()`. Built on `feature/serve-autostart`, which this branch includes.
- Tests: [`parity.test.ts`](editors/vscode/src/test/suite/parity.test.ts) (settings scopes and precedence, live changes, every command against a mock `bragi serve`).

**Checks:** extension `npm run lint` and `npm test` (54 passing) under Xvfb with `env -i` and a throwaway HOME. Rust: see below.

**Left / next:** `/v1/mcp-servers` routes once `feature/mcp-client` is in main (the extension is written against them and says so when they are missing); the tool approval card; link offer, chat images and audio, reactions, per-message delete, session open and folders in the extension. Not tested against a real linked node or a real Cursor window.

## 2026-11-22: LF-15 follow-up, MCP prompts on weirdnet-mcp 0.6.1 (branch `feature/mcp-prompts`)

**What changed**
- `weirdnet-mcp` 0.6.1 in [`bragi-mcp`](crates/bragi-mcp/Cargo.toml), for its prompts support. The prompts capability is advertised now that the server has prompts.
- [`prompts.rs`](crates/bragi-mcp/src/prompts.rs), served from `Tools::prompts` and `Tools::get_prompt` in [`tools.rs`](crates/bragi-mcp/src/tools.rs):
  - `bragi_review_file` (`path`, optional `focus`): a review request for a file or a line range (`src/lib.rs:10-40`).
  - `bragi_rewrite` (`path`, `instruction`): the messages of `bragi_core::code::prompt::build`, the request Bragi's own rewrite sends, answered as `bragi-edit` blocks. MCP prompts have no system role, so the system text leads the first user message.
  - `bragi_explain` (`path` or `selection`, optional `language`).
  - `bragi_session_summary` (`session`): the session's transcript with a summary request.
  - `bragi_continue_session` (`session`, optional `prompt`): the last 40 messages as user and assistant turns (attachments named, cut-off answers marked), then the next message.
- Files go through the same rules as the code tools: inside `--root` (`resolve`), read with `code::gather`, so secrets, binaries, non-UTF-8 and oversize files are refused, and a line range past the end is refused. Bad arguments (missing, both `path` and `selection`, an unknown session, a refused file) are `-32602` with a message saying why.
- Docs: a Prompts section in the site guide [`docs_mcp.dart`](site/lib/pages/docs_mcp.dart) and a line in the [README](README.md).

**Checks**: see the merge commit's entry below once merged; `tests/protocol.rs` `prompts_list_and_get` covers the capability, `prompts/list` (names, titles, required arguments), every prompt, and the refusals (missing arguments, outside the root, a missing file, `.env`, a binary, a range past the end, `path` and `selection` together or neither, an unknown prompt, an unknown session).

**Not tested**: real MCP clients' prompt pickers.

**What's next**: none planned for the MCP server.


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

**Next:** Ymir's side is in [ymir/docs/design/video.md](https://github.com/WeirdBrains/ymir/blob/main/docs/design/video.md). The wish list goes to the Weirdnet Lead.


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
- Prompts: added on 2026-11-22 with `weirdnet-mcp` 0.6.1 (see the entry above).
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

## 2026-11-22: LF-37 MCP client design

- Design for adding outside MCP servers: [docs/design/mcp-client.md](docs/design/mcp-client.md). weirdnet-mcp 0.6.1 already has a usable client (feature `client`, `agent`); no weirdnet blocker. Wish list for the Weirdnet Lead is in the design.
- Branch `feature/mcp-client` is now merged: the client is built (store, endpoint tool-call loop, and `bragi mcp-servers`). See the LF-37 entry at the top of this log.
