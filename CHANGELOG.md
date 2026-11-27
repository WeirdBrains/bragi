# Changelog

All notable changes to Bragi are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Bragi uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- The VS Code extension does the rest of what the apps do with a chat (LF-40): files, the selection, images and sound attached in the panel (`/v1/code/chat` takes `attachments`); reactions and deleting a message with undo; opening a chat from history and chat folders; and showing a link code for another device. New `bragi serve` routes: `GET /v1/sessions/{id}`, `…/messages/{id}/react`, `DELETE …/messages/{id}`, `/v1/undo`, `/v1/folders…`, `/v1/sessions/{id}/folder`, `/v1/link/offer…`. `docs/design/extension-parity.md` has nothing left open.
- Agent mode and outside MCP servers in the VS Code extension (LF-40): `bragi serve` manages the MCP servers (`GET|POST /v1/mcp-servers`, `PUT|DELETE /v1/mcp-servers/{id}`, `…/enabled`, `…/test`, `…/always-allow`; secrets go in once and never come back) and answers approval cards (`POST /v1/agent/approve`); `/v1/code/chat` takes `agent` and `folder`, and its approval events say whether "always allow" applies. The extension adds a server's scope, edit, test and taking back "always allow", and the chat panel's **Agent** switch with approval cards (allow once, for this task, always, deny).
- Video in the phone app (LF-43, V3): attach a clip from the gallery or files with the video button; the limits chip under the composer opens a sheet for this device's override. On Android and iOS the platform decodes the frames Bragi plans for the model (no ffmpeg on phones; no sound from phone clips yet); a Flutter build on a computer uses ffmpeg. The message shows a video card that plays the clip inline (Play/Pause, a seek bar, elapsed / total, Mute) with the frames the model saw beneath it (tap one to play from there), and otherwise says why and offers **Open in system player**. A clip and an audio file never play at once, and leaving the chat stops it. New dependency: `video_player`.
- Video in the VS Code extension (LF-43, V2): attach a clip from the chat panel (**Video…**, the Explorer's context menu, a drop, or **Bragi: Attach Video to Chat…**) on the Bragi target; `bragi serve` samples it with the same limits and override as the app (`video` on `POST /v1/code/chat`, which streams a `video` event with the frames; `POST /v1/video/limits` and `/v1/video/limits/set`). The panel shows the limits chip with **Limits…** for the override, plays the clip with the webview's `<video>` (mp4, m4v, mov, webm, ogv) with the frames the model saw beneath it, and otherwise says why and offers **Open in system player**.
- Video player, V2 slice 2 (LF-43): the desktop plays an attached clip inline (ffmpeg decodes up to 15 fps at up to 960 px; the picture follows the sound, or a silent clock, and drops frames that come late), with Play/Pause, a seek bar, elapsed / total, Mute and the frames the model saw (click one to play from there). When it can't play inline (no ffmpeg, the clip isn't on this device, ffmpeg can't read it, or decoding falls behind) the card says why and offers **Open in system player**. A clip and an audio file never play at once. The terminal app shows a video bubble (`video  clip.mp4  0:42  12 frames  sound sent`) and adds `/frames` (the times and labels of the frames sent) and `/play` (the system player). The original clip is remembered by its path when `keep_original` is on, so playing works on the device that attached it.
- Video, V2 slice 1 (LF-43): a clip is sampled off the UI thread, sized from the endpoint's `/v1/models` limits when the chat goes to an endpoint. Each model's video limits show in a chip (`video ready (…)` or `video needs image cost: set limits for MODEL`), and this device's override is set with `/video-limits key=value …` / `/video-limits clear` in the terminal app or the Limits editor in the desktop composer (keys `context`, `image_tokens`, `image_patch`, `image_edge`, `max_images`, `audio_seconds`, `audio_tokens_per_minute`, `max_request_bytes`); it fills what isn't reported and only tightens what is, and says which values it ignored.
- `bragi mcp`: `bragi_video_probe` (length, size, frame rate, sound) and `bragi_video_frames` (frames as images, each after a `[mm:ss.s]` text block, times in `structuredContent`; a `count` of 1 to 64 or sized for `model`), and a `video` input on `bragi_chat`. Clips inside the project root or as base64 `data:` URLs.
- Bragi starts stdio MCP servers with a 64 MiB line cap (weirdnet-mcp's default is 4 MiB), so tool results with many images arrive. weirdnet-mcp 0.6.1 → 0.6.2.
- Video attachments, phase 1 (LF-43): `/attach clip.mp4` in the terminal app and the desktop's attach box sample the clip with ffmpeg into timestamped frames (and the soundtrack, when the model takes audio and reports its limits) sized from the model's reported limits: the pool's catalog (context and `MediaLimits`: per-image cost or patch size, edge, images per request, audio length, request bytes; weirdnet-agent 0.5.7). The core also reads an OpenAI-compatible endpoint's `weirdnet.context`/`weirdnet.limits`/`weirdnet.max_request_bytes` from `/v1/models` (`Core::endpoint_video_limits`); the terminal app and the desktop don't use it yet. There is no default frame count: when nothing reports the limits, the attach refuses and names what is missing, and a per-model override (`video-limits.json`) can set them; it fills missing fields and can only tighten reported ones. weirdnet-agent 0.5.6 → 0.5.7, weirdnet-backends → 0.4.8. Needs `ffmpeg` and `ffprobe` (`BRAGI_FFMPEG`, beside the executable, or `PATH`). No player yet: the chat shows the first frame.

## [0.15.1] - 2026-11-26

### Changed

- Device sync pulls the roster, conversations and wake cards from each device in one session (weirdnet-sync 0.8.5 `pull_types_shared`: one hello and roster catch-up instead of one per type). A type a device refuses fails alone; works with devices on 0.7.1 to 0.8.4. weirdnet-sync 0.8.4 → 0.8.5.

## [0.15.0] - 2026-11-26

### Added

- Wake-ups (B4, weirdnet-push 0.1.1): each device makes a wake handle and key and shares them, sealed as `bragi.wake`, only with your linked devices (never with cert-only servers). A wake carries no message content, and stale or repeated wakes are refused. Settings, Wake-ups: on by default with an off switch (off withdraws the card and forgets the key). "Wake through your relay" is opt-in and off by default: it keeps a connection to your home relay on `/weirdnet/wake/1` so linked devices can wake this one without Google or Apple push, and it asks first because it uses more battery. On Android it runs as a foreground service with a quiet notification, so F-Droid builds can be woken too. In the desktop window, the Android app and `bragi-tui wake [status|on|off|relay on|off]`. APNs and FCM delivery are not wired yet.
- An F-Droid build recipe that passes `fdroid lint`, with the store text and icon in fastlane metadata. F-Droid builds have no QR camera (its scanner needs Google ML Kit, which is not free software) and pair with a desktop by typed code.

## [0.14.0] - 2026-11-26

### Fixed

- A Ymir inference server linked cert-only (`--server inference`) can read your conversations again: they are sealed under a key for conversations alone while such a server is linked, and chats from before the link are re-sealed for it after a sync pass that reached every device (and moved back once no such server is left). Every Bragi device that shares conversations with one needs this release.

### Changed

- weirdnet-sync 0.8.4, weirdnet-net 0.8.3, weirdnet-agent 0.5.6 and weirdnet-backends 0.4.7.

## [0.13.0] - 2026-11-26

### Added

- A Training view on the Models screen, in both shells (desktop: a Training toggle; Flutter: the Training button): each of your Ymir model nodes' training runs (status, base, adapter, dataset and its license, loss) and resumable jobs (state, attempt, last checkpoint), and the versioned rollouts on your inference nodes with each version's share of new conversations, which one is the canary and the run that trained it. A read only; it needs Ymir 0.16.0 (`/ymir/train-status/1`) on the model nodes, and devices without it are counted, not shown. `Core::training_overview` and `bragi_core::models::training` in the core, `training_overview` in the bridge.

## [0.12.0] - 2026-11-25

### Added

- The Flutter app uploads a model from this device to one of your model nodes, like the desktop dialog (Models, then Upload a model): the file is read and hashed first, then sent with its name, license, the base for an adapter, whether the node keeps it after a restart (`node.toml`), copies to other nodes and an optional rollout on an inference node with its share of new conversations in percent (presets or typed, 0 to 100). Progress with Cancel; sending it again resumes, and the Models screen lists unfinished uploads with Resume and Forget. A phone often can't hold a multi-GB model, so the dialog also shows the same upload as a `ymirctl models upload` command for a computer that has the file (it warns above 4 GB on a phone). Bridge: `models_inspect_upload`, `models_upload`, `upload_command`, `models_pending_uploads`, `models_forget_upload`, `models_inference_nodes`, `upload_percent_presets` and the `uploaded` event; `UploadSpec::ymirctl_command` in `bragi-core`.

## [0.11.0] - 2026-11-25

### Added

- The model picker (desktop and terminal) tags a model you uploaded or trained with where it came from and its license (`upload, MIT`, `train`), straight from the pool catalog, without asking each node (Ymir's model-upload U8; needs Ymir 0.15.3 on the inference node, which fills catalog/1's `origin` and `license`). Hub models carry no tag, as on the Models screen. `ModelInfo.origin`, `license` and `origin_tag()` in `bragi-core`.

### Changed

- weirdnet-agent 0.5.4 and weirdnet-backends 0.4.5.

## [0.10.0] - 2026-11-25

### Changed

- The desktop Upload dialog's rollout share is a percentage now (Ymir's decision 8, 2026-11-25; needs Ymir 0.15.0 on the inference node): presets 0, 1, 5, 10, 20, 50 and 100%, or any whole percentage from 0 to 100 typed in. It replaces the 0/1/5/10 weight choice. The upload is added at weight 0, then given exactly that share; the other versions share the rest. An inference node on an older Ymir keeps it at weight 0 and the dialog says why and shows the `ymirctl` commands. `RolloutAsk.percent` (was `weight`; unfinished uploads saved by 0.8.0 and 0.9.0 still read).
- Copying an upload on to other nodes: a node that can't copy it from the node it went to (it can't reach that node, or runs an older Ymir) now gets the file from this device instead, as `ymirctl models upload --node all` does, and its result line says so. A node that refuses (no space, a name in use) is still reported as it is.

## [0.9.0] - 2026-11-25

### Added

- Copy an upload on to other model nodes (Ymir's model-upload plan, U6; needs Ymir 0.14.0 on them): the desktop Upload dialog has "Also copy to" for the other ready nodes. The file is sent once from this device; the others copy it from the node it went to, node to node, and each saves it in its own `node.toml` (unless "Keep it" is off). A node without room after what it keeps free is left out before anything is copied, and the result says so per node. `UploadSpec.copy_to` and `UploadedView.copies` in `bragi-core`.

## [0.8.0] - 2026-11-25

### Added

- Add an upload to a rollout (Ymir's model-upload plan, U4 follow-up; needs Ymir 0.13.0 on the inference node): the desktop Upload dialog takes an optional rollout name, the inference node that runs the rollout and a canary weight (0, 1, 5 or 10%). After the upload, Bragi adds the model as the rollout's next version over Ymir's `/ymir/rollouts/1`, saved in that node's `node.toml`, and shows the result. Raising a version above 10% stays with `ymirctl models rollout weight --eval`. If the node refuses or runs an older Ymir, the dialog says why and shows the `ymirctl` command instead. `bragi-core` has a small, temporary copy of the rollouts wire types (`models/ymir_rollouts.rs`), and the demo nodes answer rollouts too.

## [0.7.0] - 2026-11-25

### Added

- Upload a model from this device (Ymir's model-upload plan, U4; needs Ymir 0.12.0 on the model node): the desktop Models screen has an Upload dialog for a GGUF model or LoRA adapter. It shows what the file is before anything is sent (model or adapter, format, architecture, size), then takes the node, the name, an optional license, an adapter's base (one the node serves) and whether to keep it in the node's `node.toml` (on by default). Progress shows with Cancel; the node keeps what was sent, and the screen lists unfinished uploads with Resume and Forget, also after Bragi restarts. The node's refusals are shown with what to do: no space left after the free space its owner keeps (`upload_keep_free`), a name in use, an adapter for a base it doesn't serve, a Ymir too old to take uploads. An adapter for llama.cpp says it is used from the base's next start. The result shows the `ymirctl` commands that add it to a rollout. `bragi-core` has the logic (`Core::models_inspect_upload`, `models_upload`, `models_pending_uploads`, `models_forget_upload`) and the bridge passes the new event through; the Flutter dialog comes later.

### Changed

- `weirdnet-agent` 0.5.2, `weirdnet-backends` 0.4.4 and `weirdnet-model` 0.5.3. A node's stored models now carry their origin, base and license.

## [0.6.0] - 2026-11-24

### Added

- Ymir rollouts (Ymir's training plan P4b): the versions of a rollout (`support@2`, `support@3`) are folded under its name in the model list, so the picker shows one row, and picking the name lets the pool choose a version for each conversation. The terminal app lists the versions after the name so one can be pinned; a pinned version keeps its nodes and context limit.

## [0.5.0] - 2026-11-24

### Added

- Cert-only links for Ymir servers (Ymir's M10): `bragi-tui link CODE --server inference|model` links a server shown by `ymirctl pool link --cert-only` with a certificate and a service grant signed with your identity, never the identity seed. An inference server also gets the keys for the pool's conversations; a model server gets no data keys. `Core::pair_accept_confirm_server` does the same for the other shells.

### Changed

- `weirdnet-sync` 0.7.1 and `weirdnet-device-link` 0.4.2, so Bragi syncs with servers linked cert-only (including roster-only model nodes).

## [0.4.0] - 2026-11-24

### Added

- The conversation history sidebar can be hidden, and the choice is remembered. In the terminal app: F10, Ctrl-B when it is hidden, or `/sidebar`. In the desktop window: History in the title bar, or Hide on the sidebar. On a wide Flutter layout (a tablet or a desktop window), the same Hide control folds the bar away; on a phone the history stays in the drawer.
- A device can be linked to more than one Ymir pool and can switch which one chats go to. Each pool keeps its own identity, history and node addresses. Settings lists them, and can add, rename, switch or forget a pool. The terminal app also has `bragi pool list|add|switch|rename|remove` (and `/pool` in the app). Adding a pool shows the same link code as the first pairing.

## [0.3.0] - 2026-11-24

### Added

- Training consent per chat (Ymir's training plan P2): a chat is used for training only when you mark it. The phone and desktop apps have a cap button in the chat's top bar (it asks first and says what is left out), the GPUI desktop app a Training button next to Agent, and the terminal app `/train` and `/train off` (the chat's title shows `· training`). It syncs to your linked devices as Ymir's `{conversation}/train` record, and your Ymir nodes export only marked chats (`ymirctl context export --training`), on your own devices, without system prompts, attachments, tool results or reasoning. Withdrawing it takes the chat out of later exports.

### Fixed

- `set_theme_preference` in the bridge keeps any other preference by updating the two it sets (clippy's `needless_update`).

## [0.2.1] - 2026-11-23

0.2.0 was tagged but its Release workflow failed, so it has no release artifacts or GitHub release; 0.2.1 is the first release with everything listed under 0.2.0.

### Fixed

- A shell command that runs out of time or is stopped now ends at once with everything it started, on every Linux: the process group gets SIGKILL directly instead of through the `kill` program, whose `kill -KILL -PID` form fails in procps-ng 4 (Ubuntu 24.04).
- macOS: the `bragi` binary inside Bragi.app is ad-hoc signed as well as the app, so `codesign --verify` passes now that the app opens through the `bragi-app` launcher.
- Release workflow: the Linux package check lists the desktop tarball alone (`bragi-[0-9]*`), not the terminal-only `bragi-tui` tarball too.

## [0.2.0] - 2026-11-23

### Changed

- Chat and code are modes of one conversation in the terminal app, not separate screens. Tab (or Shift-Tab, `/chat`, `/code`, `/mode`) switches; the conversation, its transcript, the draft and the code context stay. A plan made in chat goes to the model with the next code request, and code turns (the ask, the answer with its Thinking, shell runs) are saved in the conversation, so chat sees them too. Code turns are marked `code` in the transcript (and on desktop and phone, which show the same conversations). The header shows which mode you are in. Ctrl-B now moves between the message box and the chats (Tab used to). Older conversations open as chat turns; nothing needs migrating.
- `bragi` is now the terminal app as well: `bragi` with no arguments, or `bragi PATH`, runs it in the same program (no hand-off to another binary), and `bragi --help` lists everything. The desktop window opens with `bragi desktop` (the desktop entry, the macOS app and the Windows Start menu shortcut do this for you). A build without the window (BSD, servers) is the same `bragi`, terminal app only. `bragi serve` and `bragi mcp` are unchanged. `bragi-tui` still works as a thin alias (a symlink to `bragi` on Linux and macOS, a small program on Windows) for one release, then goes.
- The packages keep the `bragi-tui` name for the terminal-only build: it installs `bragi` (no GUI libraries) and `bragi-tui` links to it.

### Added

- The desktop link: `bragi linker pair` shares this computer's project folders with Bragi on a phone. It shows a QR code (and a code to type) that pins the desktop's key and expires after five minutes or one use. Paired phones read and write files there, gather code context, apply and undo changes, and run shell commands as you with output streamed back and a stop button. They connect directly over the LAN or VPN or through the Weirdnet relay, end-to-end encrypted either way. Nothing outside the shared folders is reachable. `bragi linker devices`, `revoke`, `roots` and `confirm` manage it. See [docs/desktop-link.md](docs/desktop-link.md).
- Code mode in the phone app. A Chat/Code toggle switches what the message box sends in the same conversation. A project is a shared folder on a paired desktop (or a local folder on a computer). You pick files and folders for the context, review the proposed changes hunk by hunk, apply and undo them, and run shell commands with streamed output and a stop button. **Pair with desktop** scans the desktop's QR code with the camera or takes the typed code, and sets the route (Auto, Direct only, Weirdnet first). Up and Down on a hardware keyboard walk the prompt history (the chat's, or the project's), and a bell lists past notifications.
- The terminal app keeps a prompt history: Up and Down at the first or last line of the message box walk back through what you sent (and give back what you were typing), shell style, without repeats in a row. Chat has one history; code mode has one per project folder. Up to 1000 entries are kept in the profile.
- Past notifications: every toast is kept (up to 200, in the profile) and listed newest first in a window opened with F9, `/notifications` or the bell in the header, which shows how many are new. A toast under the pointer stays until the pointer leaves.
- Shell commands from the terminal app: `!cmd` or `/shell cmd` runs in the project folder, and its output and exit status are kept in the conversation. Esc stops a running command.
- `/root DIR` in code mode makes a missing folder (or fills an empty one) with a starter `README.md` and `.gitignore`; existing files are never touched. A suggestion can be asked with no files in the context (to create them).
- Folders, not only files: `/attach DIR` sends a folder's text files as one attachment (respecting `.gitignore` and the size limits), and the file picker's filter matches folders too.
- In the terminal app, every list takes the mouse: themes, the file picker and the notifications as well as chats, models and settings. A click picks a row; a click on the picked row (or a double click) uses it, and the wheel scrolls.
- A Models screen on the desktop and phone apps: search the Hugging Face Hub with a fit badge for each of your Ymir nodes, pull a model to one node, the best-fitting node or all of them with progress and cancel, see what each node has, and remove with a confirm. It runs over Weirdnet with this device's identity; nodes accept it only from devices linked to their owner.
- See and save the files in a chat, on desktop and phone. Each file attached to a message (text, image or audio) and each file an answer writes (a code block that names its file, such as ```` ```python hello.py ````, or an embedded image) gets a card under its message with its name, type and size, a preview (the first lines of a text file, or the picture), and Save. On desktop, Save opens the system save dialog. On the phone, it opens the system save sheet.
- Audio files in a chat play on their card, on desktop and phone. Each card has Play/Pause, a seek bar and the elapsed and total time, and the total shows before the clip loads. Only one clip plays at a time, and it stops when you switch chats. Phones play through the system player and audio session, so calls and other apps pause it. The desktop plays through the system audio output; with no sound device, the controls still work, silently. On Linux the desktop app now needs ALSA (libasound).
- `bragi-tui`, Bragi in a terminal, locally or over SSH. It links with a code, QR code or node address, and has sessions in nested folders, OpenAI-compatible endpoints in Settings, and a picker for where chats go and the model. Chat streams with stop and continue, replies render as Markdown with highlighted code, and the Thinking section folds open. It takes text, image and audio attachments and shows node status. Keyboard shortcuts are listed on a help screen, and copying goes through OSC 52.
- Code suggestions and rewrites in the terminal app. It takes files and folders as context (respecting `.gitignore`) and rewrites a file or line range. Each hunk of the diff (unified or side by side) can be accepted or rejected, the write can be undone, and `bragi-tui code` does the same from scripts.
- The terminal app has a new look, themed with weirdnet-themes: Bragi's own palette by default, light and dark, any of the crate's themes with `--theme`, and `NO_COLOR` and ASCII fallbacks. It has rounded panes, bubbles for your messages, a status bar, windows over a dimmed screen, toasts, a styled diff view and a QR code beside the link code.
- `bragi-tui` ships in the Linux, macOS and Windows downloads, and as its own deb, rpm and Arch package without GUI libraries.
- `bragi serve`: a local API on 127.0.0.1 for editors and other local tools (token-protected), also
  available from the desktop app with `bragi --serve`, and as the headless `bragi-serve` binary.
- A VS Code and Cursor extension (`editors/vscode`): inline code suggestions, rewrites with a diff
  to accept or reject, and a chat panel that uses the editor's context.
- Rewrites in the editor extension are reviewed hunk by hunk: Accept and Reject over each hunk, Accept All and Reject All, and the editor's undo takes an accepted hunk back. The hunks come from `bragi serve` (new `/v1/code/hunks` and `/v1/code/apply` routes), the same `bragi_core::code` hunks the terminal app reviews.

### Fixed

- The phone app's theme editor no longer rebuilds during a build when it opens, and its Saved message stays in view however far the colours are scrolled.
- Code mode in the terminal app no longer clears itself: every ask, answer and shell run stays in one scrollable transcript, and each answer's Thinking is kept, folded to its title line, and opens with a click (Ctrl-K still opens or folds them all).
- Thinking from an agent task is kept with its answer in the history.
- Code mode fits its prompt to the model's context window: files that don't fit are left out (and named), and a "request exceeds the available context size" refusal is retried once with fewer files or shown as a plain message instead of raw JSON.

## [0.1.1] - 2026-11-21

### Added

- Replies render as Markdown on the desktop and the phone: bold, italic and strikethrough, headings, lists and task lists, quotes, tables, links (opened in your browser, address shown on hover), rules, inline code, and code blocks with highlighting, their language and a Copy button. It renders as the answer streams, without flicker or broken half-open code blocks. Each reply has a Copy button for its raw Markdown. Raw HTML stays text and images are never fetched.
- A Thinking section above each answer from a reasoning model, on desktop and phone. The model's reasoning streams into it live with the time it takes, it folds away when the answer starts, and you can open it again. Before anything arrives, the reply says it is waiting. Reasoning comes from servers that send it apart from the answer (llama.cpp, vLLM) or from a `<think>` block at the start of the text (Qwen3 and similar). It is kept with the message and is not sent back to the model.
- Delete from your history: a single message (hover or long-press it; deleting a question asks whether to delete its answer too), a whole conversation from the history list, or all history from Settings behind a typed confirm. Message and conversation deletes can be undone for a few seconds.
- Deletes reach your other linked devices with the next sync, and each device removes the deleted messages, their attachments and older versions from its own stored history. Deleted messages are never sent to a model again.
- Emoji reactions on any message, the model's replies too, on desktop and phone: a quick bar (👍 👎 ❤️ 😂 🎉 🤔) and a picker (hover on desktop, long-press on the phone), shown as small pills under the message. Click a reaction again to remove it. They are saved with the conversation, sync to your other devices and go when their message is deleted. The next turn tells the model about them in a short note, such as "[The user reacted to this reply with 👎]". They are not part of Copy.

### Fixed

- Links carry node addresses, so Bragi finds inference nodes that discovery misses.
- Paste a node address into Link a device on desktop and mobile; it is kept and dialed at once.
- A clearer message when no inference node is found, saying what to check.
- Desktop has a Copy button for the link code and Ctrl+Shift+V to paste one.
- The Link a device node notice clears itself or times out with advice, and its placeholder fits.
- The chat scrolls with a fixed composer, sticks to the latest message and offers Jump to latest.

## [0.1.0] - 2026-11-20

### Added

- Chat with the models on your own [Ymir](https://github.com/WeirdBrains/ymir) cloud, with streaming, stop and continue.
- Device linking with link codes and QR codes, and history that syncs between your devices.
- Desktop app for Linux, macOS and Windows, and a phone app for Android and iOS.
- Bragi and Ymir accept each other's link codes.
