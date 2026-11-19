# Bragi

Bragi is the chat app for your own cloud. It links to your Weirdnet identity like any of your
devices, finds your [Ymir](https://github.com/WeirdBrains/ymir) inference nodes, and streams answers
from the models on your own machines. Nothing goes through a hosted service.

Status: **B1** (see "Bragi milestones" in Ymir's `docs/DESIGN.md`). Pairing, the catalog from one
inference node, streaming chat with stop and continue, and local history, on Linux (GPUI) and
Android (Flutter). Nothing is released yet.

## Layout

| Path | What |
| --- | --- |
| `crates/bragi-core` | Everything Bragi does: identity and pairing (`weirdnet-device-link`), the `weirdnet-agent` pool client, chat events, history. Also `demo`, an in-process inference node with a test identity for tests and demos. |
| `crates/bragi-desktop` | The desktop shell (GPUI), binary `bragi`. |
| `crates/bragi-bridge` | `flutter_rust_bridge` API over `bragi-core`. |
| `app/` | The Flutter app (Android in B1). |
| `scripts/desktop-scenario.sh` | Plays the end-to-end scenario in the desktop shell inside Xvfb and takes screenshots. |
| `third_party/` | Patched copies of published crates (see its README). |

## Building

```sh
cargo test --workspace                       # bragi-core's scenario over the memory transport
cargo run -p bragi-desktop -- --demo         # the desktop shell against the demo node
scripts/desktop-scenario.sh /tmp/shots       # the scenario, headless, with screenshots

cd app
flutter_rust_bridge_codegen generate         # after changing crates/bragi-bridge/src/api
BRAGI_BRIDGE_LIB=$CARGO_TARGET_DIR/debug/libbragi_bridge.so flutter test
flutter build apk                            # add --dart-define=BRAGI_DEMO=true for the demo
```

GPUI needs its usual Linux build dependencies (Vulkan, Wayland and X11 headers, fontconfig). The
Flutter app builds `bragi-bridge` through the Native Assets hook (`app/hook/build.dart`), so the
Android SDK and NDK and the Rust Android targets in `crates/bragi-bridge/rust-toolchain.toml` are
needed.

## License

BSD-3-Clause; see `LICENSE`. `third_party/libp2p-swarm` is MIT, from libp2p.
