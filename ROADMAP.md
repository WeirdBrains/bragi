# Bragi roadmap

<!-- Generated from roadmap.toml by site/tool/gen_roadmap.py. Edit roadmap.toml, not this file. -->

Bragi is the chat client for a Ymir cloud. Every milestone ships its features in the shared Rust core for both the desktop and phone shells, so neither falls behind. The plan lives in Ymir's design doc.

Updated 2026-11-19. Design: [docs/DESIGN.md](https://github.com/WeirdBrains/ymir/blob/main/docs/DESIGN.md#bragi-milestones).

## B1 Core and both shells (Done)

Pair with your own devices, list the models your cloud offers, stream a chat, stop it and continue it, and keep local history. Works in the Linux desktop app (GPUI) and the Android app (Flutter), both on one shared Rust core.

Weirdnet crates: `weirdnet-agent`, `weirdnet-device-link`, `weirdnet-sync`, `weirdnet-net`, `weirdnet-relay`.

Known gaps:
- Tested against a simulated inference node, not yet a real Ymir node or across real machines.
- Continue sends the partial answer plus an instruction, so a real model may rephrase at the seam.
- Link codes are pasted, with no QR scanner yet. Nothing is released or on a store.

## B2 Pool client (Planned)

Use several inference nodes at once: one combined model list, retries, and automatic mid-answer failover, with the stand-in and cache flags shown in both shells.

Weirdnet crates: `weirdnet-agent`.

Known gaps:
- Needs Ymir's several-model-nodes work (M4) on the other side.

## B3 Platforms (Planned)

iOS (TestFlight), macOS and Windows, and Android packaging for the apk, aab and F-Droid.

Weirdnet crates: none new.

## B4 Sync and optional extras (Planned)

Chat history on the shared context store, so a conversation follows you between devices. Optional: an on-device model and push wake-ups.

Weirdnet crates: `weirdnet-sync`, `weirdnet-push`.

Known gaps:
- Waits on Ymir's context sync (M5). `weirdnet-push` is planned and does not exist yet.

## B5 Breadth (Planned)

BSDs where the desktop toolkit builds, agent mode and attachments, on both shells.

Weirdnet crates: `weirdnet-agent`.
