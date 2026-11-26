# Video: attaching, understanding and playing clips in Bragi

Status: design, post-v1, 2026-11-22 (LF-38). V1 started 2026-11-26 (LF-43, branch `feature/video-upload`): `bragi-core::video`
(limits from the pool catalog's `MediaLimits` (weirdnet-agent 0.5.7) or an endpoint's `/v1/models` `weirdnet.*`, budget, planner,
`FfmpegCli`, `FakeSource`, the per-model override that fills gaps and only tightens), `Media::Video`, `Core::attach_video`, and attach in the
terminal app and the desktop. V2 slice 1 (branch `feature/video-v2-mcp-override`): sampling off the UI thread with the endpoint's limits when
the chat goes to an endpoint, the limits chip and per-model override editor (`/video-limits` in the terminal app, Limits in the desktop composer),
`bragi-mcp` `bragi_video_probe`, `bragi_video_frames` and `video` on `bragi_chat` (weirdnet-mcp 0.6.2). V2 slice 2 (branch `feature/video-v2-player`): the desktop's inline player (`bragi-core::video_play`
decodes MJPEG frames from ffmpeg at up to 15 fps and 960 px, the picture follows the audio player's clock and drops late frames, a silent
clock for clips without sound; Play/Pause, seek, Mute, the frames the model saw as a strip that plays from there) with a fallback to the system
player when there is no ffmpeg, the clip isn't on this device, ffmpeg can't decode it or decoding falls 1.5 s behind for 3 s; the terminal
app's `/frames` and `/play` (system player). The original clip is remembered by its path (`Sampled.clip`, with `keep_original`), not copied.
V2 slice 3 (branch `feature/video-v2-vscode`): the VS Code extension attaches a clip (file dialog,
Explorer menu, a drop on the chat panel) and `bragi serve` samples it (`video` on `/v1/code/chat`, a `video` event with the frames,
`/v1/video/limits` and `/v1/video/limits/set` for the chip and the override); the chat webview plays it with `<video>` (only the played
clips' folders are local resource roots; `media-src` limited to the webview), the frames strip seeks, and a missing clip, a format the
webview won't try (mkv, avi) or a playback error shows why and **Open in system player**. Not yet: the phone player (V3), transcription,
packaging ffmpeg. None of it blocks v1. Ymir's side (the
server and node half, and the native-video phase) is in [`ymir/docs/design/video.md`](https://github.com/WeirdBrains/ymir/blob/main/docs/design/video.md).
Written for Weird Brains LLC by m3talsmith's agents.

## Goal

Attach a video to a message and have any vision model answer about it, play the clip in the chat next to the
audio player, in all three front ends. Later, hand the whole clip to models that read video natively.

## Which front end ships where

Checked in the repo on 2026-11-22 (README, `scripts/dist-*`, `packaging/`): the desktop is GPUI and only the phones are Flutter.

| Platform | Front end | Evidence |
|---|---|---|
| Linux | **GPUI** (`bragi-desktop`, binary `bragi`) plus the terminal app (`bragi-tui`) | `scripts/dist-linux.sh` builds `-p bragi-desktop -p bragi-tui` |
| macOS | **GPUI** plus the terminal app | `scripts/dist-macos.sh`, same packages, per target |
| Windows | **GPUI** plus the terminal app | `scripts/dist-windows.ps1`, same packages |
| Android | **Flutter** (`app/`, over `bragi-bridge`) | `scripts/dist-android.sh` (apk, aab) |
| iOS | **Flutter** | `scripts/dist-ios.sh` |
| BSDs | terminal app only (GPUI does not build there yet) | Ymir `docs/DESIGN.md` |

The VS Code extension (`editors/vscode`) is separate and runs wherever VS Code does. Flutter is therefore **not** a desktop front end: no
Linux or Windows Flutter plugin question exists, and the design below has Flutter on Android and iOS only. (The Flutter app can be built for
desktop for tests, but nothing ships that way.)

## Decisions (2026-11-22, m3talsmith)

1. **The desktop (GPUI) gets an inline player, with a fallback to the system player** when inline playback cannot handle the clip (codec, size,
   no decoder). Decided; it is no longer an open question.
2. **No default frame count or frame size, in Bragi or in Ymir.** How many frames, how large, and how long a soundtrack come from what the
   chosen model supports. Ymir reports the limits; Bragi plans frames to fit. See "Fitting the clip to the model".

## What exists today

| Piece | State |
|---|---|
| Content parts | `weirdnet-agent` 0.5 carries a message's content as text or parts: text, `image_url` (data URL) and `input_audio` (`ContentPart::image`, `ContentPart::audio`). `Modalities` has `VISION` and `AUDIO` only. **No video part and no video modality.** |
| Attachments | [`attach.rs`](../../crates/bragi-core/src/attach.rs): `Media::Image { mime, data }` and `Media::Audio { format, data }`, base64, kept with the message so later turns and other devices see what the model saw. Limits: 5 MiB per image, 10 MiB per audio file, 5 attachments per message, WAV and MP3 only. `Media::needs()` returns the modality the chosen model must have, and the send is refused up front if the model lacks it. |
| Audio player | [`audio.rs`](../../crates/bragi-core/src/audio.rs) in core: `Deck` (one clip at a time, stops when the chat changes), `clock` and `seek_ms`, optional `decode` for shells that play through their own output. The desktop shell plays through its own output; Flutter plays through the platform (`app/lib/src/audio.dart`, `ClipBackend`, `PlatformClipBackend`, a scripted backend for tests). The core's design follows Mouser's `mouser-media`. |
| Routing | Ymir sends a request with images or audio only to a node whose copy of the model takes them (per-model `Capabilities`, catalog/1 `inputs`). A model that lacks a modality gets a 400 ("does not take images"). |
| Backends | llama.cpp (with a multimodal projector) takes images, and audio on models whose projector has it (Gemma 4 E4B does, Qwen3.6-35B-A3B does not). It has no video input. |
| Request size | Media travels inside the infer/2 request as base64 (about 4/3 of the file). That is why the limits are small. |

## The problem

- There is no standard video part in the OpenAI-style API. Some servers invent `video_url`; they disagree on the details.
- llama.cpp takes no video. Models that read video (Qwen-VL family, some Gemma and Gemini-class models) do it by being fed frames plus timestamps, or through a backend's own video input.
- A clip is big: ten seconds at 1080p is several MiB, a minute is tens. It cannot ride in one request like a picture.

So the first version has to turn a clip into things that already travel: images and audio.

## Design

### Two phases

1. **Phase 1, frames plus soundtrack (any vision model).** A clip becomes a short ordered list of image parts, each labelled
   with its timestamp, plus the soundtrack as one audio part. No new part type, no new protocol, no new capability. It
   works today on every vision model Ymir runs, and on a node with no ffmpeg anywhere near it.
2. **Phase 2, native video.** A new `video` modality and part in the shared types, a capability flag in model metadata,
   and Ymir passing the clip through to backends that take it (see the Ymir doc). Phase 1 stays as the fallback for
   models without the flag, so the user never sees a difference except quality.

### Where the sampling happens

Options weighed:

| Where | For | Against |
|---|---|---|
| **Bragi (client), chosen for phase 1** | The request stays small and already-supported; frames are chosen on the user's device, so the raw clip never leaves it unless the user wants it to (privacy); no ffmpeg needed on any node; the stored message keeps exactly what the model saw (as for images now); works with every pool Bragi can reach, including plain OpenAI endpoints. | Every Bragi platform needs a decoder, including phones. (The chosen planner needs the model's limits from Ymir; see "Fitting the clip to the model".) Other OpenAI clients (Cursor, Claude Desktop through `ymirctl mcp`, scripts) get nothing. |
| Ymir bridge (server) | Every client of the OpenAI endpoint benefits from a `video_url` part; one ffmpeg, not many. | The clip must travel whole (size, privacy); the node does CPU-heavy decoding on a machine meant for inference; changes what a Ymir node must have installed. |
| Both, same code | | Needs the shared crate below. |

Decision: **Bragi samples first.** The same sampling code is then reused by Ymir's bridge as an opt-in later (Ymir
doc, "Server-side expansion"), which is why the logic belongs in one `weirdnet-media` crate (wish list) and not in either app.

### The sampling pipeline

`bragi-core::video` (new module, next to `attach` and `audio`; the shells only call it):

1. **Probe**: container, duration, width, height, frame rate, audio track present or not. Refuse unreadable, DRM-protected or zero-length files with a clear error.
2. **Choose timestamps.** One plan, three strategies, all producing a list of times:
   Every count below is bounded by the **frame budget** (next section), never by a built-in number.
   - `uniform` (the default mode): as many evenly spaced frames as the budget allows, at most the user's optional `fps` if set. Spread evenly across the duration instead of truncating the end.
   - `scene`: frames at scene changes (ffmpeg's `select='gt(scene,T)'`, threshold default 0.3, a detector setting and not a frame count), always including the first frame and, when budget allows, the last; if there are fewer than the budget, top up uniformly; if more, keep the strongest changes.
   - `hybrid`: a uniform backbone with part of the budget, plus scene changes with the rest, de-duplicated within 250 ms.
3. **Extract and shrink.** Decode only those frames (seek, don't decode everything), scale to the size the budget gives (the model's own image size where it reports one, else the largest that fits), re-encode as JPEG (quality 80) or PNG for screen recordings with flat colour (detected by palette size).
4. **Label.** Each frame becomes two parts: a text part `[00:07.5]` and the image part. A leading text part says what the clip is: `Video "name.mp4", 00:42, 6 frames sampled, with sound.` The timestamps let the model say *when* things happen.
5. **Soundtrack.** If the clip has audio, extract it as mono 16 kHz WAV, capped by what the model's context or audio limit allows. Then:
   - the chosen model takes audio (`Modalities::AUDIO`): send it as an `input_audio` part;
   - the model does not: **transcribe** it with a speech-to-text model if the pool has one (a Whisper-class model on a node, or a local one), and add the transcript as text parts with timestamps (`[00:03] "…"`). If nothing can transcribe, say so in the leading text part (`sound not included`) and do not fail;
   - the user can switch the soundtrack off.
6. **Budget.** See the next section. The estimate is shown in the attach chip before sending.

The result is plain `Media::Image` and `Media::Audio` items plus text, so storage, history, sync between devices and
routing need **no change**. A new `Media::Video { name, duration_ms, frames: Vec<Frame>, audio: Option<..> }` is *only* a
UI-side grouping so the chat shows one video bubble instead of eight thumbnails; the wire form is the expanded parts.

### Fitting the clip to the model

There is no default frame count, frame size or soundtrack length. The plan is derived from the model the message will go to, using
metadata Ymir reports in catalog/1 (and in `ymirctl models`); the wish list below names the fields:

| Limit | Comes from | Used for |
|---|---|---|
| Context window (tokens), minus room for the prompt, history and the reply | model metadata | the total the frames and transcript may cost |
| Per-image token cost (as a function of image size, or a fixed cost per image) | model and projector metadata | frames x cost, and the size to scale to |
| `max_images` per request (or none) | model and backend metadata | a hard ceiling on frames |
| Preferred or maximum image edge | model and projector metadata | scale size |
| Audio accepted, and its maximum length | model metadata (`Modalities::AUDIO`, a limit) | soundtrack as audio, its cap, or a transcript |
| Native video limits (seconds, bytes, frame rate) | model metadata, phase 2 | whether to send the clip whole |
| Request size the bridge accepts | Ymir (`max_request_bytes`) | a last cap on the bytes sent |

The planner: `frames = min(max_images, (context_budget - text_and_audio_cost) / image_cost)`, then the size is chosen so that frames
fit the byte limit too, then the timestamps are placed by the chosen mode. If that gives fewer than 2 frames for a clip longer than a
couple of seconds, the attach says plainly that this model has room for only that much and what the user can do (a model with a longer context, a
shorter clip, or a trimmed range), instead of sending one poor frame. A short range can be picked with start and end handles, and then
the budget is spent on that range. The user may *lower* any number (fewer frames, smaller images); they cannot raise one past what the model reports.

**When the metadata is missing** (a node older than the field, a plain OpenAI endpoint, a model never described): Bragi **refuses to
sample, with a clear message, and offers an explicit per-model override.** The message names what is missing (`this model's image cost and
context size aren't reported`), says that images alone still work, and offers "set limits for this model" (context tokens, tokens per image,
max images), stored per model on that device and shown in the chip as a user setting.

Why refuse and not guess, and not probe:
- A guess is a hidden default under another name. Too high gives an opaque 400 or a silently truncated prompt after the user waited for
  extraction and upload; too low wastes the model's ability and the user cannot tell why answers are thin.
- Probing (send growing requests until one fails) costs real tokens and time, may be billed on a third-party endpoint, can fail for reasons other
  than limits, and is not repeatable. Not chosen.
- Asking Ymir already happens first: for a pool model the answer comes from the catalog and nothing is asked of the user. Refusing only
  applies when the answer does not exist, and the override gives the user's own, visible number, which is not a default.
- Single images keep working as today (they have fixed limits already), so a missing field never breaks what exists.

### Settings

Per attach (a sheet on the attach chip) with preferences in settings and in the config file. None of these sets a frame count or size; they only narrow what the model's limits allow:

| Setting | Default | Notes |
|---|---|---|
| `video.mode` | `uniform` | `uniform`, `scene`, `hybrid` |
| `video.fps` | unset | optional ceiling in frames per second; unset means the budget decides |
| `video.max_frames` | unset | optional, may only lower the model's limit |
| `video.scene_threshold` | `0.3` | |
| `video.max_edge` | unset | optional, may only lower the model's size |
| `video.audio` | `auto` | `auto` (audio part, else transcript), `transcript`, `off` |
| `video.range` | whole clip | start and end, set per attach |
| `video.limits.<model>` | none | the explicit override for a model with no reported metadata |
| `video.keep_original` | `true` | keep the clip file with the message for playback, else keep only the frames |

### Limits

- Clip: at most 500 MiB read from disk and at most 10 minutes, refused with a message that says what to do (trim it). The limit is on what we *read*, not on what we send.
- What travels is bounded by frames and audio, not by the clip: the frames and soundtrack the plan produces are sized to fit the request limit Ymir reports (`max_request_bytes`) as well as the model's limits. If the total would still exceed it, drop frames evenly, then shorten the audio, in that order. The 5 MiB image and 10 MiB audio limits still apply to each part.
- `MAX_PER_MESSAGE` counts a video as **one** attachment, and phase 1 takes one video per message (the budget is per request, and two clips would split it).
- Stored with the message: frames and transcript always (they are what the model saw); the original clip only if `keep_original` is on (needed for the player), under a size cap, stored in the same attachment store the audio clips use, and deleted with the message.
- Extraction runs with a time limit (60 s plus the clip's duration), on a background thread, with a cancel, and a progress line (`Reading frames 5/N`, N from the plan).

### Playback

The player mirrors the audio player's split: **the core decides, the shell draws.**

- `bragi-core::video::Deck` is the audio `Deck` generalised: one media clip plays at a time (audio or video, so a video stops a playing voice note and the other way round), and it stops when the chat on screen changes. Probably the same type with a `Kind`.
- Time keeping (`clock`, `seek_ms`) is reused as is. The audio clock stays the master clock for a video that has sound (picture follows sound, dropping frames), and a wall clock when it has none.
- **Desktop shell (GPUI), Linux, macOS and Windows: an inline player, decided 2026-11-22.** Decode through ffmpeg (the `decode` pattern, a `video-decode` feature) into frames drawn as a texture, with the audio output as the master clock. Controls: play and pause, seek bar, elapsed and total, mute, fullscreen, and a "frames the model saw" strip (click a frame to jump there). **Fallback to the system player** (`xdg-open`, `open`, `start`) when inline playback can't handle the clip: no decoder for its codec, a size or frame rate the decoder can't keep up with (measured on the first seconds, not guessed), or ffmpeg missing. The chat then shows the thumbnail strip and an "Open in system player" button, and says why. The same fallback is used if the player fails mid-clip.
- **Flutter app, Android and iOS only**: a `VideoClipBackend` beside `ClipBackend` (`app/lib/src/audio.dart`), over the platform player (the `video_player` plugin, which supports both phones' native players). Same controls, the frame strip, and a scripted backend for tests like the audio one. There is no Flutter desktop player to build.
- **TUI**: no inline playback. The chat shows a bubble `video  name.mp4  00:42  N frames  sound` and the frame strip as small half-block or kitty-graphics images where the terminal supports it (detected the way the rest of the TUI does), else just the line. `Enter` on it opens the clip in the system player (`xdg-open`, `open`, `start`) and `f` shows the sampled frames list with timestamps. The TUI **attach** is the file picker (LF-34) accepting video extensions.
- **VS Code extension** (`editors/vscode`): attach through the file dialog or drag and drop onto the chat webview; the webview plays the clip with an HTML5 `<video>` element (VS Code's webview is Chromium, so mp4/H.264 and webm usually play, other codecs show the thumbnail strip and "open externally"). Sampling stays in `bragi-core`, reached through the bridge or the `bragi serve` local server (the extension already talks to it), so the extension never needs its own ffmpeg.
- **Android and iOS**: attach from the gallery or camera; the platform's decoders do the sampling (MediaMetadataRetriever / AVAssetImageGenerator through a Flutter plugin or a small FFI shim, behind `bragi-bridge`), so no ffmpeg is bundled on phones. Playback through the platform player.

### Attach and chat UI

- Attach picks up video extensions (`mp4`, `m4v`, `mov`, `webm`, `mkv`, `avi`, `gif` as a video when it has more than one frame and is over a few hundred KiB, else it remains an image). Detection is by content probe, not extension alone.
- Chip: thumbnail, name, duration, estimated token cost, and a gear for the settings sheet. A removed chip deletes the extracted temp files.
- If the chosen model is not a vision model, the existing "this model doesn't read images" refusal is shown, with a one-tap switch to a model in the pool that does (the routing already knows which).
- In the chat: the video bubble (player), then "frames the model saw" collapsed under it.
- The existing capability check (`Media::needs`) gets the union of VISION (and AUDIO when the soundtrack goes as audio, not as a transcript).

### The ffmpeg dependency

Bragi should not require users to install ffmpeg, and should not link it in carelessly (licence, size, platform spread).

| Option | Notes |
|---|---|
| **Shell out to a bundled `ffmpeg` and `ffprobe`** (chosen for desktops) | The simplest; process isolation (a crashing decoder doesn't crash Bragi, and a hostile file can't corrupt our memory); easy time limits and cancel. Use an LGPL build (no `--enable-gpl`, no x264/x265, no `--enable-nonfree`); decoding only plus JPEG/PNG/WAV encoding, which keeps it small (a minimal build is on the order of 5 to 10 MiB per platform). Found in this order: `BRAGI_FFMPEG`, the copy beside the executable, then `PATH`. |
| Link libav* via `ffmpeg-next` | One process, but build pain on every platform, dynamic-link licence obligations (LGPL needs a relinkable form), and a memory-safety surface in-process. Not chosen. |
| Pure Rust decoders | `symphonia` for audio is fine already; video codecs (H.264, VP9, AV1) in pure Rust are not complete enough. Revisit later; AV1 and VP9 only might become possible. |
| Platform decoders | Used on Android and iOS (and Media Foundation on Windows, AVFoundation on macOS as a later fallback). Behind the same `FrameSource` trait in the core, so the sampling plan is shared. |

A `FrameSource` trait (`probe`, `frames_at(times)`, `audio(range)`) hides which one runs, with an `FfmpegCli` implementation and platform ones. The tests use a `FakeSource` that returns generated frames, so no test needs ffmpeg.

### Platform packaging

Per shared AGENTS.md, Bragi ships on every platform. How video lands on each (desktops are GPUI plus the terminal app, phones are Flutter):

| Platform | Package | ffmpeg |
|---|---|---|
| Linux (deb, rpm, Arch, AppImage/tarball, Flatpak, Nix) | nfpm, `packaging/arch`, `packaging/nix` | Package depends on the distro `ffmpeg` (a `Recommends`, so Bragi installs without it and the attach says how to get it); the tarball and AppImage bundle the LGPL build. Flatpak has the ffmpeg extension. |
| macOS (dmg) | `packaging/macos` | Bundled in `Contents/Helpers/`, signed and notarised with the app (an unsigned helper breaks notarisation, so it is signed in the same step; no keys in the repo). |
| Windows (msi/exe) | `packaging/windows` | Bundled beside `bragi.exe`; per-user install as before. |
| FreeBSD, OpenBSD, GhostBSD, DragonFly | `pkg` / tar.gz | Depend on the port `ffmpeg`; tar.gz users install it or set `BRAGI_FFMPEG`. |
| Android (apk, aab, F-Droid) | | No ffmpeg. Platform decoders only. F-Droid builds can't carry a proprietary or non-reproducible binary, so this also keeps the F-Droid build clean. |
| iOS (ipa) | | No ffmpeg (App Store rules and size). AVFoundation. |

Where ffmpeg is missing, video attach is hidden or disabled with one sentence of how to get it. Nothing else in Bragi changes. The LGPL notice and the exact build configuration are listed in the repo's third-party notices, and the build script that produces the bundled binary lives in `scripts/` and is reproducible (pinned source tarball and checksum).

### MCP exposure

- `bragi mcp` (the server): `chat` already takes images and audio. It gains a `video` input (a path or base64, with the same sampling settings), expanded through the same code, so Cursor, VS Code and Claude Desktop get video understanding through Bragi. Path inputs are subject to the server's existing root and read restrictions (no reading arbitrary files; the MCP client must name a file the user allowed). A read-only tool `video_probe` (duration, size, streams) is cheap and safe. Frame extraction as a tool (`video_frames` returning images and timestamps) is useful on its own, and is read-only.
  - Built (V2 slice 1) as `bragi_video_probe` and `bragi_video_frames` (both read-only, files inside the project root or a base64 `data:` URL written to a private temp folder). `bragi_video_frames` takes a `count` (1 to 64, the caller's) or sizes frames for `model` from its reported limits (refusing with what to set, as attach does); `max_edge`, `mode`, `start_ms`, `end_ms`. weirdnet-mcp 0.6.2's image block has no annotations, so each image follows a text block with its time (`[00:07.5]`) and `structuredContent.frames[]` carries `at_ms`, `label` and the mime type. One call returns at most 32 MiB of frames.
  - `bragi_chat` takes `video` plus `video_max_frames`, `video_max_edge`, `video_mode`, `video_start_ms`, `video_end_ms`; the clip counts as one of the five files.
- Bragi as an MCP *client* (see the MCP client design, `docs/design/mcp-client.md`, LF-37): stdio servers are started with a 64 MiB line cap (`StdioTransport::spawn_limited`; weirdnet-mcp's default is 4 MiB), so a batch of frames arrives whole. A tool result that carries an image content block already needs to reach the model (weirdnet-mcp 0.6.2's `McpSource` still turns results into text: `call_tool`, not `call_tool_content`; a Weirdnet ask); a video result from an outside tool is a path or URL, handled as an attachment via the same sampling (never fetched silently: a URL is shown and needs approval).
- Tool-call approvals treat anything that reads a file or fetches a URL for video as a file-read-tier action.

### Privacy and safety

- Sampling is local. The raw clip is not uploaded in phase 1, only the frames and sound the user sees in the chip, and the chip shows how many and what the soundtrack will be before sending.
- Frames come from the clip, so what is on screen goes to the model, wherever the pool runs. The usual "your pool, your data" rule applies; for a pool that includes a remote or third-party OpenAI endpoint, the attach says so before sending (as for images).
- Strip metadata: frames are re-encoded, so EXIF and GPS go; the container's metadata (location, device) is never sent. The file name goes into the leading text part only, and the user can blank it.
- Transcribing the soundtrack with a model sends the audio to that model's node; the same rule applies.
- Temp files live in the app's private cache with restricted permissions, are deleted when the send finishes or is cancelled, and at startup for leftovers.
- Hostile input: decode in a child process with a time limit and memory cap where the platform allows, no network access for the child (`-protocol_whitelist file,pipe`, so a playlist or `concat` file can't make ffmpeg read other files or hit the network), and refuse non-media containers. Treat the clip's subtitles and metadata as untrusted text and don't pass them to the model unless the user asks.
- Prompt-injection note: text inside frames or in the audio can try to instruct the model. Nothing special beyond what already holds for images, but agent mode's approval cards are the safeguard, and a video in a chat that has tools enabled shows a one-line reminder.

### Testing

- Core unit tests with `FakeSource`: timestamp planning for every mode (cap respected, first frame always included, even spread, short clips, one-frame clips, no audio track), budget reduction, limits and refusals, the leading text and timestamp labels, and the soundtrack branches (audio part, transcript, neither). Property tests: output times are sorted, unique and inside the duration.
- Capability tests with the demo inference node: a video to a text-only model is refused with the switch suggestion; to a vision model without audio it goes as frames plus a transcript; to one with audio it goes with `input_audio`.
- `FfmpegCli` tests against tiny generated fixtures (a few KiB, made at test time with ffmpeg `lavfi`, so none are checked in), marked `#[ignore]` unless ffmpeg is present, and run on CI where it is installed. A hostile-file test: a text file named `.mp4`, a truncated file, a `concat` playlist referencing another file.
- Player tests like the audio ones: `Deck` single-clip rule across audio and video, the scripted Flutter backend, the desktop fallback to the system player when decoding fails, seeking, stop on chat change.
- Flutter widget tests (demo mode, phones) and GPUI view tests for the chip, the settings sheet and the video bubble; the TUI scripted test for the bubble and the frame list; an extension test for drag and drop and the webview message.
- A live check, before release of the feature: a real clip on real vision models through a real pool (Gemma 4 E4B, Qwen3.6-35B-A3B) and a check that the answer cites the right timestamps.
- Never in tests: the real network, a real identity, a model download.

### Milestones

1. **V1**: `FrameSource` with `FfmpegCli`, the sampling plan, `Media::Video` grouping, attach in `bragi-core` and `bragi-mcp` (`video` input, `video_probe`, `video_frames`), settings. No player yet; the chat shows the frame strip.
2. **V2** (built 2026-11-26): GPUI desktop inline player with the system-player fallback, and its attach UI; TUI bubble, picker and open-externally; the VS Code attach and webview player.
3. **V3**: Flutter (Android and iOS) player and platform decoders.
4. **V4**: native video when Ymir's capability flag exists (Ymir doc, phase 2): the sampler becomes the fallback.
5. Packaging for each platform is part of V1 (bundled ffmpeg builds), not an afterthought.

### Open questions

- GPUI's way to draw video frames and keep them in sync with the audio clock (a texture per frame, or the platform's video layer): spike first. The system-player fallback covers the gap while it is built.
- Which models' metadata the first release can fill in: it depends on Ymir reporting the limits (Ymir doc, milestone 1). Until then video is refused with the clear message and the override, and single images are unaffected.
- Whether a speech-to-text model joins the standard model catalog (it needs a place in `weirdnet-model`).

(Decided 2026-11-22 and removed from here: desktop inline player with a system-player fallback; no default frame count.)

## Weirdnet wish list

Bragi does not create or modify `weirdnet-*` crates. For the Weirdnet Lead, in order of use:

1. **`weirdnet-media` (new crate).** The generic half of this design: `probe`, the `FrameSource` trait with an ffmpeg-CLI implementation, `plan_frames(duration, Budget) -> Vec<Time>` (uniform, scene, hybrid; the `Budget` is derived from the model's reported limits, with no built-in count) and `Budget::from_limits(limits)`, frame scaling and JPEG/PNG encoding, soundtrack extraction to mono 16 kHz WAV, hardened ffmpeg invocation (`protocol_whitelist`, time and memory limits, cancel), locating the binary (`FFMPEG` env var, beside the executable, `PATH`), and a fake source for tests. Mouser's `mouser-media` is the nearest existing code; the player's clock logic (`Deck`, `clock`, `seek_ms`) now lives in Bragi core and is also generic.
2. **`weirdnet-agent`: a video part and modality.** `ContentPart::video(..)`, `Modalities::VIDEO`, `infer/2` carrying it, and the capability flag in `Capabilities` and catalog `inputs` (Ymir doc, phase 2). Until then Bragi uses image and audio parts only. A part that carries a content-root reference (not inline bytes) would also fix the size problem for good (this is the same gap as in LF-12).
3. **`weirdnet-openai`: `video_url` handling** in the OpenAI bridge: accept `video_url` (data URL) and map it to the video part when the model takes it, else expand it to frames and sound through `weirdnet-media` when the operator turns that on.
4. **`weirdnet-model`: speech-to-text and video capability metadata.** A Whisper-class model in the catalog with a `transcribe` role, and per-model limit fields: context window, per-image token cost (by size), `max_images`, preferred image edge, audio maximum length and native video limits. These are what the frame budget is computed from; there is no fallback default, so a model without them gets the refuse-with-override path.
5. **`weirdnet-backends`: a transcription call** (audio in, text with timestamps out) next to chat, embeddings and the rest, plus passing a video part to backends that take one.
6. ~~**`weirdnet-mcp`: image and audio content blocks in tool results**~~ (0.6.2: `Content::from_json`, `decoded_bytes`, `Client::call_tool_content`). Still open: `McpSource` (feature `agent`) passing image and audio blocks to the model instead of text, and annotations (or `_meta`) on image blocks for the frame time.
7. **Bundling help:** an LGPL ffmpeg build recipe or crate (`weirdnet-ffmpeg`?) that every WeirdBrains app can ship, with the notices and a reproducible build, so Mouser, Morph and Bragi do not each carry one.
