# Bragi for VS Code and Cursor

Code suggestions, rewrites and chat in your editor, answered by the models on your own devices.
The extension talks to the Bragi app on the same computer, and Bragi sends each request to your
Weirdnet pool (your Ymir nodes). One VSIX works in VS Code, Cursor and other VS Code-based editors
that install from a VSIX or Open VSX.

## What it does

- **Inline suggestions** as you type (ghost text; Tab accepts). Bragi sees the code around the
  cursor and the file's problems.
- **Rewrite selection or file** (`Bragi: Rewrite Selection…`, Ctrl+Alt+R / Cmd+Alt+R): say how
  the code should change, review the change in a diff, then accept (✓, Ctrl+Alt+Enter) or reject
  (✕, Ctrl+Alt+Backspace).
- **Chat panel** (the Bragi icon in the activity bar) that can include the current file, the
  selection, other open files and the file's problems. Code blocks in answers have Insert and Copy.
  On the Bragi target, chats are kept in Bragi's history like any other.
- **Target and model picker** in the status bar: through Bragi, your Weirdnet pool (any model it
  lists, or its default) or an endpoint from Bragi's Settings; or a direct OpenAI-compatible server.

## Set up

1. Install the VSIX: in VS Code or Cursor, run **Extensions: Install from VSIX…** and pick
   `bragi-0.2.0.vsix`, or run `code --install-extension bragi-0.2.0.vsix`
   (`cursor --install-extension bragi-0.2.0.vsix` in Cursor).
2. Start Bragi's local API on the same computer, either:
   - the desktop app with `bragi --serve`, which shares the app's link and history, or
   - without a window: `bragi serve` (or the `bragi-serve` binary on a headless machine).

   It listens on `127.0.0.1:4877` only. Use one or the other for a Bragi profile, not both.
3. The extension picks up the API token from the Bragi profile on first use
   (`~/.local/share/bragi/serve/token`, or under `$BRAGI_HOME`) and keeps it in the editor's secret
   storage. If your profile is somewhere else, run **Bragi: Set Bragi API Token** and paste the
   output of `bragi serve token`, or set `bragi.tokenFile`.

The Bragi app has to be linked to your Weirdnet already; **Bragi: Show Connection Status** says
which models it can reach.

## Settings

| Setting | Default | |
| --- | --- | --- |
| `bragi.target` | `bragi` | `bragi` (the local app) or `direct` (an OpenAI-compatible server). |
| `bragi.serveUrl` | `http://127.0.0.1:4877` | The local API. Loopback addresses only. |
| `bragi.model` | empty | The model on the Bragi target; empty is Bragi's default. |
| `bragi.bragiTarget` | `null` | Where Bragi sends requests: `null` for Bragi's default, or a target picked with **Bragi: Choose Target and Model** (the Weirdnet pool, or an endpoint and model from Bragi's Settings). |
| `bragi.direct.url` | empty | An OpenAI-compatible base URL, e.g. a Ymir bridge `http://127.0.0.1:8484/v1`. Its key: **Bragi: Set Direct Endpoint API Key** (secret storage). |
| `bragi.direct.model` | empty | Empty uses the server's first model. |
| `bragi.fallbackToDirect` | `false` | Use the direct server when the local API isn't running. |
| `bragi.inline.enabled` | `true` | Inline suggestions on or off (also **Bragi: Toggle Inline Suggestions**). |
| `bragi.inline.debounceMs` | `350` | How long typing must pause before a suggestion is asked for. |
| `bragi.context.openFiles` | `true` | Send other open files with rewrites and chat. |
| `bragi.context.diagnostics` | `true` | Send the file's problems. |

## Privacy and security

Requests go only to the local Bragi API (loopback, token-protected, and closed to web pages) or
to the direct server you set. Bragi then sends them over Weirdnet to your own nodes. Nothing goes
to Weird Brains. Inline suggestions are off in untrusted workspaces.

## The local API

`bragi serve` speaks JSON over HTTP with `Authorization: Bearer <token>`:

- `GET /v1/health`, `GET /v1/state`, `GET /v1/targets` (the pool and Settings endpoints; requests
  take an optional `target`)
- `POST /v1/code/suggest`, `POST /v1/code/rewrite`, `POST /v1/code/chat` (server-sent events),
  `POST /v1/cancel`
- `GET /v1/models` and `POST /v1/chat/completions`, OpenAI-compatible, so other local tools can
  use Bragi too.

The prompts, the reading of code out of answers and the diffs live in `bragi-core`
(`crates/bragi-core/src/code.rs`), shared with Bragi's terminal app. The direct target uses a
TypeScript copy (`src/prompts.ts`) kept in step with it.

## Develop

```sh
cd editors/vscode
npm ci
npm run compile
# Tests run in a downloaded VS Code with a throwaway profile. On Linux, headless:
env -i HOME="$HOME" PATH="$PATH" XDG_RUNTIME_DIR="$(mktemp -d)" xvfb-run -a npm test
npm run package   # bragi-0.2.0.vsix
```

Not yet: full Visual Studio (a separate VSIX format and API), and Marketplace or Open VSX listings.
