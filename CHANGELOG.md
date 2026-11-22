# Changelog

All notable changes to Bragi are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Bragi uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Delete from your history: a single message (hover or long-press it; deleting a question asks whether to delete its answer too), a whole conversation from the history list, or all history from Settings behind a typed confirm. Message and conversation deletes can be undone for a few seconds.
- Deletes reach your other linked devices with the next sync, and each device removes the deleted messages, their attachments and older versions from its own stored history. Deleted messages are never sent to a model again.

## [0.1.0] - 2026-11-20

### Added

- Chat with the models on your own [Ymir](https://github.com/WeirdBrains/ymir) cloud, with streaming, stop and continue.
- Device linking with link codes and QR codes, and history that syncs between your devices.
- Desktop app for Linux, macOS and Windows, and a phone app for Android and iOS.
- Bragi and Ymir accept each other's link codes.
