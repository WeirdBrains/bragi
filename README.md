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

Status: **B1, B2 and B6 done; B3, B4, B5 and B7 in progress** (see [Bragi milestones](https://github.com/WeirdBrains/ymir/blob/main/docs/DESIGN.md#bragi-milestones) in Ymir's design doc). Pairing, the catalog from your inference nodes, streaming chat with stop and continue, and history that syncs between your devices, on Linux, macOS and Windows (GPUI) and on Android and iOS (Flutter). A device can hide the history bar (the choice is remembered) and can switch between more than one Ymir pool. Released: [v0.4.0](https://github.com/WeirdBrains/bragi/releases/tag/v0.4.0) ([downloads](https://bragi.weirdbrains.com/download/)).

## Layout

| Path | What |
| --- | --- |
| `crates/bragi-core` | Everything Bragi does: identity and pairing (`weirdnet-device-link`), the `weirdnet-agent` pool client, chat events, history on the shared context store. Also `demo`, an in-process inference node with a test identity for tests and demos. |
| `crates/bragi-desktop` | The desktop shell (GPUI), binary `bragi`. |
| `crates/bragi-tui` | The terminal shell (ratatui), binary `bragi-tui`: the desktop's features in a terminal, locally or over SSH, plus code suggestions and rewrites. |
| `crates/bragi-mcp` | `bragi mcp`: Bragi as an MCP server (stdio, or loopback HTTP with a token) on `weirdnet-mcp`. |
| `crates/bragi-bridge` | `flutter_rust_bridge` API over `bragi-core`. |
| `app/` | The Flutter app (Android and iOS). |
| `brand/`, [BRAND.md](https://github.com/WeirdBrains/bragi/blob/main/BRAND.md) | Logo, icons, palette and usage rules. |
| `site/`, `roadmap.toml`, [ROADMAP.md](https://github.com/WeirdBrains/bragi/blob/main/ROADMAP.md) | The website (Jaspr) and the roadmap it is generated from. |
| `scripts/desktop-scenario.sh` | Plays the end-to-end scenario in the desktop shell inside Xvfb and takes screenshots. |
| `third_party/` | Patched copies of published crates (see its README). |

## Building

```sh
cargo test --workspace                       # bragi-core's scenario over the memory transport
cargo run -p bragi-desktop -- desktop --demo # the desktop window against the demo node
cargo run -p bragi-desktop -- --demo         # `bragi` alone is the terminal app (also `cargo run -p bragi-tui`)
cargo run -p bragi-tui -- --demo             # the terminal shell against the demo node (F1 lists the keys)
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
them all on a release tag. Releases go through `scripts/weirdnet_release.py`; see [docs/RELEASES.md](https://github.com/WeirdBrains/bragi/blob/main/docs/RELEASES.md).

## The terminal app

`bragi-tui` makes sure `bragi serve` (the local API the editor extension uses) is running when it
starts, in the background, and shows its state in the status bar. A server that is already running
is reused and never stopped; one it starts stops after 30 idle minutes. `--no-serve` or
`BRAGI_TUI_SERVE=off` turns that off; `BRAGI_SERVE_PORT` and `BRAGI_HOME` pick the port and the
Bragi home (the token's folder, shared with the extension and the desktop app).

`bragi-tui` is a device of its own: its profile is in `~/.local/share/bragi-tui` (or `--home DIR`,
or `BRAGI_TUI_HOME`), never the desktop app's. Link it once with `bragi-tui link`, which prints a code
and a QR code to open on a device that's already linked (`ymirctl pool add CODE` on a Ymir node). It
works over SSH. Copying (Ctrl-Y, or `c` on the link screen) uses OSC 52, so the text lands on your
local clipboard.

It has what the desktop has: linking with a code, QR code or node address; more than one saved Ymir
pool, switched from Settings or `bragi pool`; a history bar you can hide (F10, remembered); sessions in nested folders;
OpenAI-compatible endpoints in Settings (Ctrl-P) and a picker for where chats go and the model (Ctrl-O);
streaming chat with stop and continue; replies drawn as Markdown with highlighted code; the Thinking
section (Ctrl-K); text, image and audio attachments; and node status (Ctrl-T). Code mode (Ctrl-E) adds
files and folders as context, respecting `.gitignore`. It asks for suggestions, or a rewrite of a file
or line range (`/rewrite FILE:A-B how`), and shows a unified or side-by-side diff. You accept (`y`) or
reject (`n`) each hunk, `w` writes the ones you kept, and `/undo` reverts the write.

```sh
bragi-tui                                                  # the full-screen app
bragi-tui .                                                # code mode on the current folder (also `bragi .`)
bragi-tui src/main.rs                                      # on one file, rooted at its git repository
bragi-tui --path serve                                     # explicit path, for a folder named like a command
bragi-tui --no-serve                                        # without starting `bragi serve` for the editors
bragi serve ensure                                         # start `bragi serve` in the background if none is running
bragi-tui link /ip4/…/tcp/4848/p2p/…                       # dial a node Bragi can't find by itself
bragi pool list                                            # saved Ymir pools (also: add, switch, rename, remove)
bragi-tui link 'weirdnet-ymir://link/…' --server inference  # link a Ymir server cert-only, without your seed
bragi-tui code --rewrite src/lib.rs:10-40 "use iterators"  # prints a unified diff
bragi-tui code --suggest --context src "find bugs" --apply # writes the change
bragi-tui undo                                             # puts the last written change back
```

Give it a path and it opens code mode there: a folder is the root and the file picker opens on it (the
context starts empty); a file opens
from its git repository (or its folder). `.`, `..`, `~` and relative paths work, and a missing path is an
error. A lone word that is a command (`link`, `status`, `undo`, `code`, `tui`) stays a command; write
`./link` or `--path link` for a folder of that name. `bragi PATH` (the desktop binary) hands over to `bragi-tui`
the same way. Suggestions, rewrites and hunk review never leave the root.

The file picker (`f`, Ctrl-F or `/files` in code mode) is a tree of the root: `.gitignore` is respected and
`target/`, `node_modules/`, `.git/` and the like are hidden. ↑↓ or `j k` move, `→ l` and `← h` open and shut
folders, Space adds or removes the file (on a folder, all its files), Enter opens a folder or adds a file and
closes the picker, `/` filters (fuzzy), Ctrl-U clears the filter, `c` empties the context and Esc closes it.
A marker shows what is in the context, and the bottom line shows its size against the limit (64 files,
256 KiB a file, 512 KiB in all).

The prompts, the diffs and the undo journal are `bragi_core::code`, shared with the editor extension.

## Code mode on a phone (the desktop link)

A phone has no project folder or shell, so Bragi on the phone runs code mode against a desktop.
On the desktop, `bragi linker pair` shows a QR code. Scan it in the phone app with **Pair with
desktop**, or type the code shown under it. From then on, the phone reads and writes files in the
folders the desktop shares, and runs shell commands there as you, with their output streamed back.
It connects directly over your LAN or VPN, or through the Weirdnet relay when it is away. Both
routes are encrypted end to end, and nothing outside the shared folders is reachable.

```sh
bragi linker roots add ~/src/myapp   # share a folder (or --root DIR on pair/serve)
bragi linker pair                    # QR code and typed code; keeps serving (Enter: a new code)
bragi linker devices                 # paired phones
bragi linker revoke ID               # unpair one
bragi linker confirm on              # shell commands from a phone wait for a yes here
```

[docs/desktop-link.md](docs/desktop-link.md) describes the pairing, the routes and the protocol.

## MCP server

`bragi mcp` serves Bragi over the [Model Context Protocol](https://modelcontextprotocol.io) so Cursor,
Claude Desktop, VS Code and other clients can chat through Bragi, read and file sessions, get code
suggestions and rewrites as unified diffs (`bragi_core::code`, as in the terminal app), and list
endpoints and models. It uses the app's Bragi folder, so its chats land in the same history. Stdio by
default; `--http` serves `http://127.0.0.1:4878/mcp` (loopback only) with the bearer token from
`bragi mcp token`. The tools that write into the project (apply or undo a code change, save a chat's
file) are offered only with `--allow-writes`, and the tools that delete only with `--allow-deletes`. For Cursor
(`.cursor/mcp.json`):

```json
{ "mcpServers": { "bragi": { "command": "bragi", "args": ["mcp", "--root", "${workspaceFolder}"] } } }
```

Prompts (`bragi_review_file`, `bragi_rewrite`, `bragi_explain`, `bragi_session_summary`,
`bragi_continue_session`) fill in requests for the client's own model. Claude Desktop and VS Code setups, the tools, the prompts and the resources: [MCP server guide](https://bragi.weirdbrains.com/docs/guides/mcp/).
`cargo run -p bragi-mcp -- --demo` tries it against the demo node.

## License

BSD-3-Clause; see [LICENSE](https://github.com/WeirdBrains/bragi/blob/main/LICENSE). `third_party/libp2p-swarm` is MIT, from libp2p.
