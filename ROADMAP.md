# Bragi roadmap

<!-- Generated from roadmap.toml by site/tool/gen_roadmap.py. Edit roadmap.toml, not this file. -->

Bragi is the chat client for a Ymir cloud. Every milestone ships its features in the shared Rust core for both the desktop and phone shells, so neither falls behind. The plan lives in Ymir's design doc.

Updated 2026-11-19. Design: [docs/DESIGN.md](https://github.com/WeirdBrains/ymir/blob/main/docs/DESIGN.md#bragi-milestones).

## B1 Core and both shells (Done)

Pair with your own devices, list the models your cloud offers, stream a chat, stop it and continue it, and keep local history. Works in the Linux desktop app (GPUI) and the Android app (Flutter), both on one shared Rust core.

Weirdnet crates: [`weirdnet-agent`](https://crates.io/crates/weirdnet-agent), [`weirdnet-device-link`](https://crates.io/crates/weirdnet-device-link), [`weirdnet-sync`](https://crates.io/crates/weirdnet-sync), [`weirdnet-net`](https://crates.io/crates/weirdnet-net), [`weirdnet-relay`](https://crates.io/crates/weirdnet-relay).

## B2 Pool client (Done)

Use several inference nodes at once: one combined model list, retries, and automatic mid-answer failover, with the stand-in and cache flags shown in both shells.

Weirdnet crates: [`weirdnet-agent`](https://crates.io/crates/weirdnet-agent).

## B3 Platforms (In progress)

iOS (TestFlight), macOS and Windows, and Android packaging for the apk, aab and F-Droid. Android release builds, F-Droid metadata and a Linux tarball are built; macOS, Windows and iOS build in CI (unsigned) but have not been run on a machine.

Weirdnet crates: none new.

## B4 Sync and optional extras (In progress)

Chat history on the shared context store, so a conversation follows you between devices: built and shown on both shells with test identities. Optional on-device model and push wake-ups are not built.

Weirdnet crates: [`weirdnet-sync`](https://crates.io/crates/weirdnet-sync), `weirdnet-push`.

## B5 Breadth (In progress)

Text attachments and agent mode (tool calls with approval cards) are built and shown on both shells against the demo node. BSD support is blocked: the FreeBSD CI job ran and the desktop app does not build there (a GPUI bug).

Weirdnet crates: [`weirdnet-agent`](https://crates.io/crates/weirdnet-agent).
