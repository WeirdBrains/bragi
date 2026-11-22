# Changelog

All notable changes to Bragi are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Bragi uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `bragi serve`: a local API on 127.0.0.1 for editors and other local tools (token-protected), also
  available from the desktop app with `bragi --serve`, and as the headless `bragi-serve` binary.
- A VS Code and Cursor extension (`editors/vscode`): inline code suggestions, rewrites with a diff
  to accept or reject, and a chat panel that uses the editor's context.
- `bragi-tui`, Bragi in a terminal (also over SSH): link with a code, QR code or node address, streaming chat with stop and continue, model picker, node status, keyboard shortcuts with a help screen, and copying through OSC 52.
- Code suggestions and rewrites in the terminal app: files and folders as context (respecting `.gitignore`), a rewrite of a file or line range, a reviewable diff with per-hunk accept and reject, writes with undo, and `bragi-tui code` for scripts.
- `bragi-tui` ships in the Linux, macOS and Windows downloads, and as a `bragi-tui` deb, rpm and Arch package without GUI libraries.

## [0.1.0] - 2026-11-20

### Added

- Chat with the models on your own [Ymir](https://github.com/WeirdBrains/ymir) cloud, with streaming, stop and continue.
- Device linking with link codes and QR codes, and history that syncs between your devices.
- Desktop app for Linux, macOS and Windows, and a phone app for Android and iOS.
- Bragi and Ymir accept each other's link codes.
