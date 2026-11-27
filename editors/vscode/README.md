# Bragi for VS Code and Cursor

Code suggestions, rewrites and chat in your editor, answered by the models on your own devices.
The extension talks to the Bragi app on the same computer, and Bragi sends each request to your
Weirdnet pool (your Ymir nodes). One VSIX works in VS Code, Cursor and other VS Code-based editors
that install from a VSIX or Open VSX.

## What it does

- **Inline suggestions** as you type (ghost text; Tab accepts). Bragi sees the code around the
  cursor and the file's problems.
- **Rewrite selection or file** (`Bragi: Rewrite Selection…`, Ctrl+Alt+R / Cmd+Alt+R): say how
  the code should change, then review it hunk by hunk. Each hunk has Accept and Reject above it
  in the editor (or Ctrl+Alt+Y / Ctrl+Alt+N for the hunk at the cursor), its removed lines are
  marked and a hover shows the change, and the whole proposal is in a diff beside it. Accept All
  (Ctrl+Alt+Enter) and Reject All (Ctrl+Alt+Backspace) finish it at once. Accepting is an
  ordinary edit, so the editor's undo takes it back and the hunk is there to review again; your
  own edits while reviewing go into the proposal too, so they never show up as hunks.
- **Chat panel** (the Bragi icon in the activity bar) that can include the current file, the
  selection, other open files and the file's problems. Code blocks in answers have Insert and Copy.
  On the Bragi target, chats are kept in Bragi's history like any other.
- **Video in the chat** (Bragi target): **Video…** in the panel, **Bragi: Attach Video to Chat…**,
  the Explorer's context menu on a clip, or a drop on the panel (hold Shift when dragging from the
  Explorer). Bragi samples the clip into timestamped frames sized from the model's reported limits
  (it needs `ffmpeg` where Bragi runs; the extension never runs it). A chip shows whether the model's
  limits are known; **Limits…** (or **Bragi: Set Video Limits for the Model…**) sets this device's
  override, which fills what isn't reported and only lowers what is. The chat plays the clip with
  the webview's own player (mp4, m4v, mov, webm, ogv), with the frames the model saw under it (click
  one to play from there). When the clip is gone, is a format the webview can't play, or fails to
  play, the panel says why and offers **Open in system player**.
- **Target and model picker** in the status bar: through Bragi, your Weirdnet pool (any model it
  lists, or its default) or an endpoint from Bragi's Settings; or a direct OpenAI-compatible server.

## Set up

1. Install the VSIX: in VS Code or Cursor, run **Extensions: Install from VSIX…** and pick
   `bragi-0.2.0.vsix`, or run `code --install-extension bragi-0.2.0.vsix`
   (`cursor --install-extension bragi-0.2.0.vsix` in Cursor).
2. Bragi's local API starts by itself. When the extension starts and nothing answers on
   `bragi.serveUrl`, it runs `bragi serve` in the background (found on the PATH, or at
   `bragi.path`), waits until it answers and reads the token. A server it starts stops itself
   after 30 idle minutes and starts again on the next request. One that is already running is
   used and never stopped. The status bar shows the server's state; click it, or run
   **Bragi: Start/Restart Server**, to start it (or restart one this editor started). Turn the
   autostart off with `bragi.autoStartServer`. If `bragi` isn't found you get a message with a
   shortcut to the `bragi.path` setting.

   To run it yourself instead: the desktop app with `bragi --serve` (shares the app's link and
   history), or without a window `bragi serve` (the `bragi-serve` binary on a headless machine).
   It listens on `127.0.0.1:4877` only. Use one or the other for a Bragi profile, not both.
3. The extension picks up the API token from the Bragi profile on first use
   (`~/.local/share/bragi/serve/token`, or under `$BRAGI_HOME`) and keeps it in the editor's secret
   storage. If your profile is somewhere else, run **Bragi: Set Bragi API Token** and paste the
   output of `bragi serve token`, or set `bragi.tokenFile`.

The Bragi app has to be linked to your Weirdnet already; **Bragi: Show Connection Status** says
which models it can reach.

## Setting Bragi up from the editor

Everything the Bragi apps set up, the extension can too. Run the commands from the palette, or use
the **Connections** view in the Bragi activity bar (inline buttons on each row).

| What | Commands |
| --- | --- |
| Links to inference nodes | **Link to an Inference Node…** (paste the link code, compare the short code on both screens, confirm), **Add an Inference Node by Address…**, **Remove Link…** |
| OpenAI-compatible connections | **Add / Edit / Test / Remove OpenAI Connection…**. The key goes to Bragi once, which keeps it in its own secret store; the editor never keeps it and it is never shown again. These are the same connections the Bragi apps list. The older single *direct* server (`bragi.direct.*`) keeps its key in the editor's secret storage. |
| Chats | **Delete Chats…**; the chat panel's chats are in Bragi's history. |
| Local server | **Start/Restart Server**, **Stop Server** (only one this editor started), **Show Server Status** |
| Bragi as an MCP server | **Start MCP Server** (runs `bragi mcp` in a terminal), **Stop MCP Server**, **Copy MCP Client Config…** (Cursor, VS Code, Claude Desktop) |
| Outside MCP servers | **Add MCP Server…** (and where its tools may be used: every chat, or code chats in this folder), **Turn MCP Server On or Off…**, **Test MCP Server**, **Remove MCP Server…**, **Take Back "Always Allow" for an MCP Tool…**, **Apply MCP Servers from Settings**. With an older Bragi the commands say so. |
| Chat files | **File…** and **Selection** in the panel, **Attach File to Chat…** (Explorer), **Attach Selection to Chat** (editor), or drop files: text, images (PNG, JPEG, GIF, WebP) and sound (WAV, MP3). Images and sound need the Bragi target. |
| Reactions and delete | Under each message: quick reactions, and delete (with its answer, for a question) with **Undo**. |
| Chats and folders | **Open Chat from History…**, **Delete Chats…**, **Move Chat to Folder…**, **New / Rename / Move / Delete Chat Folder…** |
| Link another device | **Link This Bragi to Another Device (Show a Code)…**: enter the code on a device that is already linked, compare the short code, confirm. |
| Agent mode | The chat panel's **Agent** switch: the model may use tools, and each call shows a card (**Allow once**, **Allow for this task**, **Always allow** for an outside MCP server's tool, **Deny**). Bragi target only. |

### Settings, user and workspace

Settings are in the Settings UI under **Bragi: Connection, Models, Chat, MCP and Advanced**
(`Preferences: Open Settings`, search `@ext:weirdbrains.bragi`), and in `settings.json` as `bragi.*`
with completion and descriptions. **Bragi: Open Settings (JSON)…** opens the user or the workspace file.
The value used is the first of: workspace folder, workspace, user (global), Bragi's default.
**Bragi: Show Settings and Where They Come From…** lists every key, its value and which of those it came from.
Keys that start a program or point at a server (`bragi.path`, `bragi.serveUrl`, `bragi.autoStartServer`,
`bragi.tokenFile`, `bragi.direct.url`, `bragi.mcp.*`) are machine scope: only your user settings count, so an
untrusted repository can't redirect Bragi. Changes apply at once, without reloading.
Secrets (the API token, keys) are never settings: they are in the editor's secret storage or Bragi's own.

`bragi.mcp.clientServers` lists outside MCP servers to add to Bragi (no secrets; add env values and tokens
with **Add MCP Server…**):

```json
"bragi.mcp.clientServers": [
  { "name": "Files", "transport": "stdio", "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "."] }
]
```

## Settings

| Setting | Default | |
| --- | --- | --- |
| `bragi.target` | `bragi` | `bragi` (the local app) or `direct` (an OpenAI-compatible server). |
| `bragi.serveUrl` | `http://127.0.0.1:4877` | The local API. Loopback addresses only. |
| `bragi.autoStartServer` | `true` | Start `bragi serve` in the background when nothing answers on `bragi.serveUrl`. Not in untrusted workspaces. |
| `bragi.path` | empty | The `bragi` program to start (a machine setting); empty looks for `bragi`, then `bragi-serve`, on the PATH. |
| `bragi.model` | empty | The model on the Bragi target; empty is Bragi's default. |
| `bragi.bragiTarget` | `null` | Where Bragi sends requests: `null` for Bragi's default, or a target picked with **Bragi: Choose Target and Model** (the Weirdnet pool, or an endpoint and model from Bragi's Settings). |
| `bragi.direct.url` | empty | An OpenAI-compatible base URL, e.g. a Ymir bridge `http://127.0.0.1:8484/v1`. Its key: **Bragi: Set Direct Endpoint API Key** (secret storage). |
| `bragi.direct.model` | empty | Empty uses the server's first model. |
| `bragi.fallbackToDirect` | `false` | Use the direct server when the local API isn't running. |
| `bragi.inline.enabled` | `true` | Inline suggestions on or off (also **Bragi: Toggle Inline Suggestions**). |
| `bragi.inline.debounceMs` | `350` | How long typing must pause before a suggestion is asked for. |
| `bragi.context.openFiles` | `true` | Send other open files with rewrites and chat. |
| `bragi.context.diagnostics` | `true` | Send the file's problems. |
| `bragi.mcp.server.allowWrites`, `bragi.mcp.server.allowDeletes` | `false` | Add `--allow-writes` / `--allow-deletes` to `bragi mcp`. |
| `bragi.mcp.server.args` | `[]` | Extra arguments for `bragi mcp`. |
| `bragi.mcp.clientServers` | `[]` | Outside MCP servers to add to Bragi (see above). |

## Privacy and security

Requests go only to the local Bragi API (loopback, token-protected, and closed to web pages) or
to the direct server you set. Bragi then sends them over Weirdnet to your own nodes. Nothing goes
to Weird Brains. Inline suggestions are off in untrusted workspaces.

## The local API

`bragi serve` speaks JSON over HTTP with `Authorization: Bearer <token>`:

- `GET /v1/health`, `GET /v1/state`, `GET /v1/targets` (the pool and Settings endpoints; requests
  take an optional `target`)
- `POST /v1/code/suggest`, `POST /v1/code/rewrite`, `POST /v1/code/chat` (server-sent events;
  `video: {path, max_frames?, max_edge?, mode?, start_ms?, end_ms?}` attaches a clip on disk and
  adds a `video` event with the frames), `POST /v1/cancel`
- `POST /v1/video/limits` (a model's reported, own and effective video limits, and the chip) and
  `POST /v1/video/limits/set` (this device's override, `key=value …`; empty clears it)
- `POST /v1/code/hunks` (the hunks between two texts) and `POST /v1/code/apply` (a text with only
  the accepted hunks), for reviewing a rewrite hunk by hunk
- Management routes behind the same token: `GET /v1/links`, `POST /v1/links/node`, `POST /v1/links/remove`,
  `POST /v1/link/accept` and `/v1/link/confirm`; `GET|POST /v1/endpoints`, `PUT|DELETE /v1/endpoints/{id}`,
  `POST /v1/endpoints/{id}/test`, `GET /v1/endpoints/{id}/models`, `POST /v1/endpoints/test`;
  `GET /v1/sessions`, `DELETE /v1/sessions/{id}`. Keys are accepted and never returned.
- `GET /v1/models` and `POST /v1/chat/completions`, OpenAI-compatible, so other local tools can
  use Bragi too.

The prompts, the reading of code out of answers, the diffs and the hunks live in `bragi-core`
(`crates/bragi-core/src/code`), shared with Bragi's terminal app, which reviews hunks the same
way. The direct target, which has no Bragi behind it, uses TypeScript copies (`src/prompts.ts`,
`src/hunks.ts`) kept in step with them.

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
