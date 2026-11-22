<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="brand/svg/bragi-lockup.svg">
    <img src="brand/svg/bragi-lockup-light.svg" alt="Bragi" width="320">
  </picture>
</p>

# Bragi

Bragi is the chat app for your own cloud. It links to your Weirdnet identity like any of your
devices, finds your [Ymir](https://github.com/WeirdBrains/ymir) inference nodes, and streams answers
from the models on your own machines. Nothing goes through a hosted service.

Status: **B5 in progress (B1 and B2 done, B3 and B4 in progress)** (see "Bragi milestones" in Ymir's `docs/DESIGN.md`). Pairing, the catalog from one
inference node, streaming chat with stop and continue, and history that syncs between your devices, on Linux (GPUI) and
Android (Flutter). Released: v0.1.0 for Linux, macOS, Windows, Android and iOS ([downloads](https://bragi.weirdbrains.com/download/)).

## Layout

| Path | What |
| --- | --- |
| `crates/bragi-core` | Everything Bragi does: identity and pairing (`weirdnet-device-link`), the `weirdnet-agent` pool client, chat events, history on the shared context store. Also `demo`, an in-process inference node with a test identity for tests and demos. |
| `crates/bragi-desktop` | The desktop shell (GPUI), binary `bragi`. |
| `crates/bragi-tui` | The terminal app (ratatui), binary `bragi-tui`: link, chat, models, node status and code suggestions and rewrites, locally or over SSH. |
| `crates/bragi-bridge` | `flutter_rust_bridge` API over `bragi-core`. |
| `app/` | The Flutter app (Android in B1). |
| `brand/`, `BRAND.md` | Logo, icons, palette and usage rules. |
| `site/`, `roadmap.toml`, `ROADMAP.md` | The website (Jaspr) and the roadmap it is generated from. |
| `scripts/desktop-scenario.sh` | Plays the end-to-end scenario in the desktop shell inside Xvfb and takes screenshots. |
| `third_party/` | Patched copies of published crates (see its README). |

## Building

```sh
cargo test --workspace                       # bragi-core's scenario over the memory transport
cargo run -p bragi-desktop -- --demo         # the desktop shell against the demo node
cargo run -p bragi-tui -- --demo             # the terminal app against the demo node (F1 lists the keys)
scripts/desktop-scenario.sh /tmp/shots       # the scenario, headless, with screenshots

cd app
flutter_rust_bridge_codegen generate         # after changing crates/bragi-bridge/src/api
BRAGI_BRIDGE_LIB=$CARGO_TARGET_DIR/debug/libbragi_bridge.so flutter test --dart-define=BRAGI_DEMO=true
flutter build apk                            # add --dart-define=BRAGI_DEMO=true for the demo
```

GPUI needs its usual Linux build dependencies (Vulkan, Wayland and X11 headers, fontconfig). The
Flutter app builds `bragi-bridge` through the Native Assets hook (`app/hook/build.dart`), so the
Android SDK and NDK and the Rust Android targets in `crates/bragi-bridge/rust-toolchain.toml` are
needed.

Packaging: `scripts/dist-linux.sh`, `dist-macos.sh`, `dist-windows.ps1`, `dist-android.sh` and
`dist-ios.sh` build each platform's release files into `dist/`, and `.github/workflows/release.yml` runs
them all on a release tag. Releases go through `scripts/weirdnet_release.py`; see `docs/RELEASES.md`.

## The terminal app

`bragi-tui` is a device of its own (profile in `~/.local/share/bragi-tui`, or `--home` /
`BRAGI_TUI_HOME`), so link it once: `bragi-tui link` prints a code (and a QR code) to open on a device
that is already linked, such as `ymirctl pool add CODE` on a Ymir node. It works over SSH; copying (Ctrl-Y,
or `c` on the link screen) uses OSC 52, so the text lands on your local clipboard.

```sh
bragi-tui                                           # chat; Ctrl-E code mode, Ctrl-L link, Ctrl-T status, F1 help
bragi-tui link /ip4/…/tcp/4848/p2p/…                # dial a node Bragi can't find by itself
bragi-tui code --rewrite src/lib.rs:10-40 "use iterators"            # prints a unified diff
bragi-tui code --suggest --context src "find bugs" --apply           # writes the change
bragi-tui undo                                      # puts the last written change back
```

In code mode, `/add PATH` adds files or folders (respecting `.gitignore`), `/rewrite FILE:A-B how` or
plain text asks, and each proposed hunk can be accepted (`y`) or rejected (`n`) before `w` writes them;
`/undo` reverts. The prompts, diffs and undo journal are `bragi_core::code`, shared with the editor
extension.

## License

BSD-3-Clause; see `LICENSE`. `third_party/libp2p-swarm` is MIT, from libp2p.
