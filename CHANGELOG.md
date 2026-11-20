# Changelog

All notable changes to Bragi are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Bragi uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Replies render as Markdown on the desktop and the phone: bold, italic and strikethrough, headings, lists and task lists, quotes, tables, links (opened in your browser, address shown on hover), rules, inline code, and code blocks with highlighting, their language and a Copy button. It renders as the answer streams, without flicker or broken half-open code blocks. Each reply has a Copy button for its raw Markdown. Raw HTML stays text and images are never fetched.
- A Thinking section above each answer from a reasoning model, on desktop and phone. The model's reasoning streams into it live with the time it takes, it folds away when the answer starts, and you can open it again. Before anything arrives, the reply says it is waiting. Reasoning comes from servers that send it apart from the answer (llama.cpp, vLLM) or from a `<think>` block at the start of the text (Qwen3 and similar). It is kept with the message and is not sent back to the model.

## [0.1.0] - 2026-11-20

### Added

- Chat with the models on your own [Ymir](https://github.com/WeirdBrains/ymir) cloud, with streaming, stop and continue.
- Device linking with link codes and QR codes, and history that syncs between your devices.
- Desktop app for Linux, macOS and Windows, and a phone app for Android and iOS.
- Bragi and Ymir accept each other's link codes.
