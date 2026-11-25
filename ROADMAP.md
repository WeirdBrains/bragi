# Bragi roadmap

<!-- Generated from roadmap.toml by site/tool/gen_roadmap.py. Edit roadmap.toml, not this file. -->

Bragi is the chat client for a Ymir cloud. Every milestone ships its features in the shared Rust core for both the desktop and phone shells, so neither falls behind. The plan lives in Ymir's design doc.

Updated 2026-11-25. Design: [docs/DESIGN.md](https://github.com/WeirdBrains/ymir/blob/main/docs/DESIGN.md#bragi-milestones).

## B1 Core and both shells (Done)

Pair with your own devices, list the models your cloud offers, stream a chat, stop it and continue it, and keep local history. Works in the Linux desktop app (GPUI) and the Android app (Flutter), both on one shared Rust core. A Ymir server can be linked cert-only (`bragi-tui link CODE --server inference`), so it gets a scoped certificate and never your identity seed (v0.5.0).

Weirdnet crates: `weirdnet-agent`, `weirdnet-device-link`, `weirdnet-sync`, `weirdnet-net`, `weirdnet-relay`.

## B2 Pool client (Done)

Use several inference nodes at once: one combined model list, retries, and automatic mid-answer failover, with the stand-in and cache flags shown in both shells. A Ymir rollout shows as one row, with its versions under it.

Weirdnet crates: `weirdnet-agent`.

## B3 Platforms (In progress)

iOS, macOS and Windows, and Android packaging for the apk, aab and F-Droid. Released in v0.1.0: Linux packages and tarballs, macOS dmg, Windows installers, Android apks and an unsigned iOS ipa. Next: store listings (Play, F-Droid, TestFlight), signing with a release key, and finishing the F-Droid draft recipe.

Weirdnet crates: none new.

## B4 Sync and optional extras (In progress)

Chat history on the shared context store, so a conversation follows you between devices: built and shown on both shells. A per-chat training consent for Ymir's training export (off unless you mark a chat, and synced like the chat) is built in every shell. Next: push wake-ups and an optional on-device model.

Weirdnet crates: `weirdnet-sync`, `weirdnet-push`.

## B5 Breadth (In progress)

Text, image and audio attachments and agent mode (tool calls with approval cards) are built on both shells. The BSD desktop window does not build yet, so on BSD it is the terminal app only.

Weirdnet crates: `weirdnet-agent`.

## B6 Your own endpoints (Done)

Any OpenAI-compatible server, with an optional API key, next to the Weirdnet pool or as a fallback when it is down. Settings add, edit, delete, test and reorder both Weirdnet links and endpoints; keys are sealed on the device and never synced. Each chat is bound to one target and keeps its own history, and chats nest in folders. Built in the core, the Flutter app (desktop and phone) and the GPUI desktop shell.

Weirdnet crates: `weirdnet-agent`.

## B7 Models on your pool (In progress)

A Models screen on both shells: search the Hugging Face Hub with a fit badge for each of your Ymir nodes, pull a model to one node, the best-fitting one or all of them with progress and cancel, see what each node has, and remove with a confirm. It runs over Weirdnet with this device's own identity, and the nodes accept it only from devices linked to their owner; a device a node refuses is shown as needing an owner device. Built in the core, the Flutter app and the GPUI desktop shell, and tried against demo nodes and a mock Hub. The desktop also uploads a GGUF model or LoRA adapter from this device to a node (Ymir 0.12.0), with progress, Cancel and Resume, an optional license, and the node's refusals such as too little free space, and can add the upload to a rollout on an inference node as its next version with a share of new conversations in percent, 0 to 100 (Ymir 0.13.0; the percentage needs Ymir 0.15.0) and have other nodes copy it node to node (Ymir 0.14.0). The model picker tags uploaded and trained models with their origin and license from the pool catalog (Ymir's U8). Next: the upload dialog in the Flutter app and a check against a real Ymir node.

Weirdnet crates: `weirdnet-backends`, `weirdnet-model`.

## v0.2 Chat, code and the desktop link (Done)

Chat and code are modes of one conversation (Tab to switch). A desktop linker with QR pairing over Weirdnet lets the phone's code mode work against a paired desktop. Also shipped in v0.2.1: project scaffolding, a shell-out command, prompt history, a notification log, mouse support in the terminal app, and thinking blocks that stay collapsed and can be expanded.

Weirdnet crates: none new.

## v0.4 History bar and more than one pool (Done)

The conversation history sidebar can be hidden, and the choice is remembered, in the terminal app, the desktop window, and on a wide Flutter layout. A device can be linked to more than one Ymir pool and can switch which one is active from Settings or `bragi pool`; each pool keeps its own history. Built in the core and on all three shells, and tried against demo nodes.

Weirdnet crates: none new.
