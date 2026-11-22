# Changelog

All notable changes to Bragi are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Bragi uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- A Models screen on the desktop and phone apps: search the Hugging Face Hub with a fit badge for each of your Ymir nodes, pull a model to one node, the best-fitting node or all of them with progress and cancel, see what each node has, and remove with a confirm. It runs over Weirdnet with this device's identity; nodes accept it only from devices linked to their owner.
- See and save the files in a chat, on desktop and phone. Each file attached to a message (text, image or audio) and each file an answer writes (a code block that names its file, such as ```` ```python hello.py ````, or an embedded image) gets a card under its message with its name, type and size, a preview (the first lines of a text file, or the picture), and Save. On desktop, Save opens the system save dialog. On the phone, it opens the system save sheet.
- `bragi-tui`, Bragi in a terminal, locally or over SSH. It links with a code, QR code or node address, and has sessions in nested folders, OpenAI-compatible endpoints in Settings, and a picker for where chats go and the model. Chat streams with stop and continue, replies render as Markdown with highlighted code, and the Thinking section folds open. It takes text, image and audio attachments and shows node status. Keyboard shortcuts are listed on a help screen, and copying goes through OSC 52.
- Code suggestions and rewrites in the terminal app. It takes files and folders as context (respecting `.gitignore`) and rewrites a file or line range. Each hunk of the diff (unified or side by side) can be accepted or rejected, the write can be undone, and `bragi-tui code` does the same from scripts.
- The terminal app has a new look, themed with weirdnet-themes: Bragi's own palette by default, light and dark, any of the crate's themes with `--theme`, and `NO_COLOR` and ASCII fallbacks. It has rounded panes, bubbles for your messages, a status bar, windows over a dimmed screen, toasts, a styled diff view and a QR code beside the link code.
- `bragi-tui` ships in the Linux, macOS and Windows downloads, and as its own deb, rpm and Arch package without GUI libraries.
- `bragi serve`: a local API on 127.0.0.1 for editors and other local tools (token-protected), also
  available from the desktop app with `bragi --serve`, and as the headless `bragi-serve` binary.
- A VS Code and Cursor extension (`editors/vscode`): inline code suggestions, rewrites with a diff
  to accept or reject, and a chat panel that uses the editor's context.

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
