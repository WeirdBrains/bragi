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

### Sessions, endpoints and the fallback

Sessions sit in nested folders and each sends to one target: your Weirdnet inference nodes, or one
OpenAI-compatible endpoint (its `/v1` and a bearer token, checked against `/v1/models` when added).
A Ymir bridge endpoint can be the Weirdnet session's fallback: used only when no inference node
answers, or to finish an answer that broke off on Weirdnet. The window shows which way the next
message goes. Tokens are kept in `secrets/<id>.token` (mode 0600) in the data folder and never
logged; TLS uses the system trust store. Endpoints and Weirdnet links (the node addresses Bragi dials)
can be edited and deleted in the same screen; edits are tested before they are saved, and deletes
ask first and never remove history. See the
[guide](https://bragi.weirdbrains.com/docs/guides/sessions/).

## Layout

| Path | What |
| --- | --- |
| `crates/bragi-core` | Everything Bragi does: identity and pairing (`weirdnet-device-link`), the `weirdnet-agent` pool client, chat events, history on the shared context store. Also `demo`, an in-process inference node with a test identity for tests and demos. |
| `crates/bragi-desktop` | The desktop shell (GPUI), binary `bragi`. |
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

## License

BSD-3-Clause; see `LICENSE`. `third_party/libp2p-swarm` is MIT, from libp2p.
