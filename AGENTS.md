# Bragi agents

The collective `AGENTS.md` in `WeirdBrains/weirdnet-agents` is mandatory for this project and wins on
conflict. Read it first. This file only adds Bragi-specific rules. Bragi's design lives with Ymir's:
`WeirdBrains/ymir`, `docs/DESIGN.md` (the Bragi sections and "Bragi milestones"); keep its status
current in the same change as the code here.

- Bragi is the client app for a Ymir cloud. `bragi-core` holds all behaviour (pairing, the catalog,
  chat, cancel, continue, history). The shells only draw `AppState` and call `Core`:
  `bragi-desktop` (GPUI) and the Flutter app through `bragi-bridge`. A shell lagging a milestone is a
  bug, not a design difference.
- Use the published `weirdnet-*` crates (`weirdnet-agent` pool client and types,
  `weirdnet-device-link`, `weirdnet-sync`, `weirdnet-net`) as they are. Never create or modify a
  `weirdnet-*` crate from this repo. Code that is generic but not published yet (the identity store
  in `bragi-core/src/identity.rs`, the GPUI text input, `bragi-bridge/src/android.rs`) is marked as
  such and listed for the Weirdnet lead.
- The URL scheme is `weirdnet-bragi://`; never bare `weirdnet://`.
- Licensed BSD-3-Clause (`LICENSE`); Rust edition 2024, `rust-version` 1.99.
- Tests use temp dirs and the in-memory transport with the demo inference node
  (`bragi_core::demo`, a test identity). They never touch the real network or a real identity.
- Nothing opens on the live desktop. Run the desktop shell and any Android emulator only in a
  virtual workspace: `scripts/desktop-scenario.sh` (Xvfb, `env -i`, a private `XDG_RUNTIME_DIR`, no
  inherited `DISPLAY` or `WAYLAND_DISPLAY`), and check the process's `/proc/<pid>/environ`.
- Desktop windows are frameless with in-app controls (close only under tiling window managers).
- On the dev box, build with `CARGO_TARGET_DIR` under a scratch directory, at `nice`/`ionice`, and clean
  `target/`, `app/build/` and `app/.dart_tool/` afterwards.
- Before committing: `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, `flutter analyze` and `flutter test` in `app/`.
