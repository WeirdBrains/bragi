# The desktop link

The desktop link lets Bragi on a phone run code mode against a project on a desktop. The phone has
no project folder or shell of its own, so the desktop serves the code tools: list, read and write
files under the folders it shares, gather files as code context, apply and undo changes, scaffold a
project, and run shell commands with their output streamed back. The model call itself still goes
from the phone to your Ymir nodes as in chat; only file and shell work goes to the desktop.

It is built on [weirdnet-net](https://crates.io/crates/weirdnet-net) app calls, the same Weirdnet
transport Bragi uses to reach Ymir. The code is [`bragi_core::link`](../crates/bragi-core/src/link/mod.rs).

## Using it

On the desktop:

```sh
bragi linker roots add ~/src/myapp     # share a project folder (repeat for more)
bragi linker pair                      # show a QR code and a typed code, then keep serving
```

On the phone: switch the chat to **Code** (the toggle in the top bar), tap **Open a project**, choose
**Pair with desktop**, and scan the QR code (or type the code under it). The camera is asked for
once. From then on, **Open a project** lists the desktop and its shared folders: pick one, and
optionally a folder inside it (a missing one is made with a starter readme). The same screen sets
the route (**Auto**, **Direct only** or **Weirdnet first**) and unpairs desktops.

In code mode the phone picks context files from the desktop folder, asks for changes in the same
conversation as the chat, reviews them hunk by hunk, applies or undoes them on the desktop, and runs
shell commands there (**Run**), with the output streamed back and a stop button. On a computer the
app can also open a local folder.

Other commands:

| Command | What it does |
| --- | --- |
| `bragi linker serve` | serve paired phones without showing a code |
| `bragi linker devices` | list paired phones, with when each was last seen |
| `bragi linker revoke ID` | unpair a phone; a running linker refuses its next call |
| `bragi linker roots [add DIR \| remove DIR]` | the folders phones may use |
| `bragi linker confirm on\|off` | whether shell commands from a phone wait for a yes on the desktop |

`serve` and `pair` take `--root DIR` (repeatable), `--port N` (default 47480, TCP and QUIC),
`--no-weirdnet` (LAN and VPN only), `--any-interface` and `--confirm-shell`. With no shared folder,
the current folder is shared.

## Routes

The desktop runs a Weirdnet node of its own, with its own key (`<profile>/linker/node.key`).

- **Direct.** It listens on LAN and VPN addresses only: private IPv4 (10/8, 172.16/12,
  192.168/16), the shared 100.64/10 range that Tailscale and similar VPNs use, and IPv6
  unique-local addresses. Container and VM bridges are skipped. `--any-interface` listens
  everywhere.
- **Weirdnet.** Unless `--no-weirdnet` is given, it holds a reservation on the Weirdnet relay, so a
  phone that is not on the same network reaches it through a relay circuit.

Both routes carry the same Noise-encrypted, mutually authenticated libp2p connection. A relay only
forwards bytes; it can't read or change them, and it can't pose as the desktop without its key.

The phone has a route setting: **Auto** (direct first, then Weirdnet), **Direct** (never through
a relay) or **Weirdnet** (the relay first, then direct). After pairing, each `hello` call refreshes
the desktop's addresses on the phone.

## Pairing

1. `bragi linker pair` makes a one-time secret (128 bits) and shows a QR code holding
   `weirdnet-bragi://desktop?v=1&p=PEER&s=SECRET&n=LABEL&x=EXPIRES&a=ADDR…&r=RELAY…`:
   - `p` is the desktop's peer id, which is its Ed25519 public key;
   - `a` lists its direct TCP addresses;
   - `r` gives indexes into the known relay list
     ([`weirdnet-device-link`](https://crates.io/crates/weirdnet-device-link)'s `KNOWN_RELAYS`).
2. The phone dials that peer id at those addresses. Noise checks that the key is the one in the
   code, so the phone has pinned the desktop's key before it sends anything. Then the phone sends
   `POST /v1/pair {secret, label}`.
3. If the secret matches, the desktop records the phone's peer id, also checked by Noise, along
   with a new 256-bit token. It answers with the token and its `hello`. The desktop stores only the
   token's BLAKE3 hash (`devices.json`, mode 0600). The phone stores the token
   (`<profile>/linker/client.json`, 0600, inside the app's private storage).
4. The code works once and for five minutes. Five wrong secrets cancel it. A new code replaces the
   old one.

**Typed code.** This is the fallback when the phone can't scan. It is 28 Crockford base32
characters in groups of four. It holds the desktop's LAN or VPN IPv4 address and port, a one-time
48-bit secret, and the first 40 bits of the BLAKE3 hash of the desktop's peer id. The phone dials
the address, checks that the key Noise shows matches that fingerprint, and only then sends the
secret. Typed codes work over the direct route only.

## Calls

Every call is a weirdnet-net app call to app `bragi-link`. The phone sends JSON bodies with
`authorization: Bearer TOKEN`. A call is served only when the token's hash and the caller's peer id
both match one paired device, so a copied token is useless from another key. Errors come back as
4xx or 5xx with `{kind, message}`.

| Call | Body | Answer |
| --- | --- | --- |
| `POST /v1/pair` | `{secret, label}` (no token) | `{device_id, token, desktop: Hello}` |
| `POST /v1/hello` | `{}` | `{version, label, roots: [{name, path}], confirm_shell, direct, relays}` |
| `POST /v1/list` | `{root, path}` | `{entries: [{name, dir, size}]}`, folders first, `.git` left out |
| `POST /v1/read` | `{root, path}` | `{text}` (`null` when missing) |
| `POST /v1/gather` | `{root, targets}` | `{files: [{path, text}], skipped: [[path, why]]}` |
| `POST /v1/apply` | `{root, changes: [{path, before, after}]}` | `{id, files}` |
| `POST /v1/undo` | `{root, force}` | `{id, files}` |
| `POST /v1/project` | `{root, path}` | `{path, note}` |
| `POST /v1/shell` | `{root, cwd, command, timeout_secs}` | a stream of frames, below |
| `POST /v1/unpair` | `{}` | `{}`: the phone removes itself |

`root` is an index into `hello.roots`. Notes on the file calls:

- `read` refuses likely secrets (`.env`, `*.pem`, `*.key` and similar), binaries and files over
  2 MiB.
- `gather` follows the same rules as code context in the terminal app: it respects `.gitignore`
  and skips hidden files, binaries, likely secrets and files over the size limits.
- `apply` writes all the changes or none: each file must still hold its `before`. It keeps an undo
  entry in the desktop's profile, never in the project.

**Shell.** The answer is newline-delimited JSON, tagged by `t`:

- `{"t":"waiting"}`: the desktop is asking its user. This repeats while it waits.
- `{"t":"out","text":…}` and `{"t":"err","text":…}`: output as it comes.
- `{"t":"alive"}`: sent every second while the command is quiet.
- `{"t":"exit","code":…,"timed_out":…,"stopped":…,"elapsed_ms":…}`: the command ended.
- `{"t":"denied","message":…}`: the desktop's user said no.

Commands run as the desktop user under `sh -c` in their own process group, with a time limit of
10 minutes by default and 4 hours at most. To stop one, the phone drops the call. The next frame
the desktop writes then fails, and the desktop kills the command and everything it started. That
takes about a second.

## Path confinement

A request's path must be relative, use `/`, and have no `..`, root or drive parts, and no NUL
bytes. The desktop then canonicalizes the deepest part of the path that exists, following
symlinks, and requires the result to stay under the canonical root. A symlink inside a project
that points outside it is refused. Undo checks again that each file it restores still resolves
inside the root.

## Files

| Where | What |
| --- | --- |
| desktop `<profile>/linker/node.key` | the link node's key (0600) |
| desktop `<profile>/linker/settings.json` | shared roots and the confirm setting |
| desktop `<profile>/linker/devices.json` | paired phones: id, label, peer id, token hash, times (0600) |
| desktop `<profile>/linker/journal/` | undo entries for changes made from a phone |
| phone `<profile>/linker/client.key` | the phone's link key (0600) |
| phone `<profile>/linker/client.json` | paired desktops, their tokens and the route setting (0600) |

On the desktop, `<profile>` is `BRAGI_TUI_HOME` or `~/.local/share/bragi-tui`. The linker folder is
mode 0700. A running linker reads `settings.json` and `devices.json` again whenever they change, so
`revoke`, `roots` and `confirm` take effect right away.
