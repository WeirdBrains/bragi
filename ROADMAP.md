# Bragi roadmap

<!-- Generated from roadmap.toml by site/tool/gen_roadmap.py. Edit roadmap.toml, not this file. -->

Bragi is the chat client for a Ymir cloud. Every milestone ships its features in the shared Rust core for both the desktop and phone shells, so neither falls behind. The plan lives in Ymir's design doc.

Updated 2026-11-20. Design: [docs/DESIGN.md](https://github.com/WeirdBrains/ymir/blob/main/docs/DESIGN.md#bragi-milestones).

## B1 Core and both shells (Done)

Pair with your own devices, list the models your cloud offers, stream a chat, stop it and continue it, and keep local history. Works in the Linux desktop app (GPUI) and the Android app (Flutter), both on one shared Rust core.

Weirdnet crates: `weirdnet-agent`, `weirdnet-device-link`, `weirdnet-sync`, `weirdnet-net`, `weirdnet-relay`.

## B2 Pool client (Done)

Use several inference nodes at once: one combined model list, retries, and automatic mid-answer failover, with the stand-in and cache flags shown in both shells.

Weirdnet crates: `weirdnet-agent`.

## B3 Platforms (In progress)

iOS, macOS and Windows, and Android packaging for the apk, aab and F-Droid. Released in v0.1.0: Linux packages and tarballs, macOS dmg, Windows installers, Android apks and an unsigned iOS ipa. Next: store listings (Play, F-Droid, TestFlight) and signing with a release key.

Weirdnet crates: none new.

## B4 Sync and optional extras (In progress)

Chat history on the shared context store, so a conversation follows you between devices: built and shown on both shells. Next: push wake-ups and an optional on-device model.

Weirdnet crates: `weirdnet-sync`, `weirdnet-push`.

## B5 Breadth (In progress)

Text attachments and agent mode (tool calls with approval cards) are built on both shells. Next: BSD support, once the desktop toolkit builds there.

Weirdnet crates: `weirdnet-agent`.
