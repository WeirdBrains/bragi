# Changelog

All notable changes to Bragi are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Bragi uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Sessions: each sends to your Weirdnet nodes or to one OpenAI-compatible endpoint, with its own model and history. Create, rename, switch, retarget and delete them; existing history becomes the default Weirdnet session.
- Folders for sessions: nested, collapsible, moved by drag and drop or a Move action. Deleting a folder asks first and moves what is in it up a level.
- Endpoints: add an OpenAI-compatible API by URL and token. Bragi checks it against `/v1/models`, tells a Ymir bridge from other APIs, and keeps the token in a private file in the data folder. Custom CAs and Tailscale certificates work through the system trust store.
- Settings for endpoints and Weirdnet links: edit an endpoint's name, URL, token (replaced, never shown), default model and fallback, and delete it after a confirm that rebinds its sessions or leaves them unbound; rename a link, change its address, or delete it. Every edit is checked with a test connection first, and no delete removes history.
- Ymir bridge fallback: the Weirdnet session uses a Ymir bridge when no inference node answers, and continues an answer there when it breaks off partway. The window shows which path a message takes, and each answer from an endpoint is marked with its name.

## [0.1.0] - 2026-11-20

### Added

- Chat with the models on your own [Ymir](https://github.com/WeirdBrains/ymir) cloud, with streaming, stop and continue.
- Device linking with link codes and QR codes, and history that syncs between your devices.
- Desktop app for Linux, macOS and Windows, and a phone app for Android and iOS.
- Bragi and Ymir accept each other's link codes.
