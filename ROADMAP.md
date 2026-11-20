# Bragi roadmap

<!-- Generated from roadmap.toml by site/tool/gen_roadmap.py. Edit roadmap.toml, not this file. -->

Bragi is the chat client for a Ymir cloud. Every milestone ships its features in the shared Rust core for both the desktop and phone shells, so neither falls behind. The plan lives in Ymir's design doc.

Updated 2026-11-19. Design: [docs/DESIGN.md](https://github.com/WeirdBrains/ymir/blob/main/docs/DESIGN.md#bragi-milestones).

## B1 Core and both shells (Done)

Pair with your own devices, list the models your cloud offers, stream a chat, stop it and continue it, and keep local history. Works in the Linux desktop app (GPUI) and the Android app (Flutter), both on one shared Rust core.

Weirdnet crates: `weirdnet-agent`, `weirdnet-device-link`, `weirdnet-sync`, `weirdnet-net`, `weirdnet-relay`.

Known gaps:
- Tested against a simulated inference node, not yet a real Ymir node or across real machines.
- Link codes are pasted, with no QR scanner yet. Nothing is released or on a store.

## B2 Pool client (Done)

Use several inference nodes at once: one combined model list, retries, and automatic mid-answer failover, with the stand-in and cache flags shown in both shells.

Weirdnet crates: `weirdnet-agent`.

Known gaps:
- Run against two simulated inference nodes only, not yet against real Ymir nodes or across machines.
- Ymir M5 nodes now resume from text already shown, but that has only been tried against a mock engine on one computer.
- Nodes are tried warm first. Speed and distance are not weighed, and you can't pick a node yourself.

## B3 Platforms (In progress)

iOS (TestFlight), macOS and Windows, and Android packaging for the apk, aab and F-Droid. Android release builds, F-Droid metadata and a Linux tarball are built; macOS, Windows and iOS build in CI (unsigned) but have not been run on a machine.

Weirdnet crates: none new.

Known gaps:
- macOS, Windows and iOS build and test in GitHub Actions (unsigned), but were never run: no scenario on macOS or Windows, and the iOS app was never launched.
- Nothing is signed with a real key or uploaded. Play, F-Droid and TestFlight need a keystore, a tagged release and Apple developer access from m3talsmith.
- The F-Droid recipe is a draft that was never run through fdroid build.

## B4 Sync and optional extras (In progress)

Chat history on the shared context store, so a conversation follows you between devices: built and shown on both shells with test identities. Optional on-device model and push wake-ups are not built.

Weirdnet crates: `weirdnet-sync`, `weirdnet-push`.

Known gaps:
- History sync was only run between simulated devices (test identities, memory transport), not between two real devices or with Morph or Ymir as the other end.
- Push wake-ups wait on the Weirdnet lead: `weirdnet-push` is planned and does not exist yet.
- The on-device model is not started: it needs a llama.cpp build and packaging on both platforms.
- Requests still send whole messages; Bragi does not yet hand a Ymir node a conversation id.

## B5 Breadth (Planned)

BSDs where the desktop toolkit builds, agent mode and attachments, on both shells.

Weirdnet crates: `weirdnet-agent`.
