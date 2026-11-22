# Extension parity: the VS Code and Cursor extension does what Bragi does

Status: design, 2026-11-22 (LF-40). Owner: [m3talsmith](https://github.com/m3talsmith).

The extension (`editors/vscode`) today is a code assistant: inline suggestions, rewrites reviewed hunk
by hunk, and a chat panel. It talks to the local `bragi serve` API on loopback, or to one direct
OpenAI-compatible server. Everything about *setting Bragi up* (links, endpoints, MCP) is done in the
apps. The goal: the extension can do all of that too, and its settings show up in the VS Code and
Cursor Settings UI and obey user and workspace `settings.json`.

## Principles

1. **One source of truth.** Links, endpoints, MCP servers and history belong to Bragi's home
   (`$BRAGI_HOME`, else the app data folder), shared with the apps. The extension manages them through
   `bragi serve`, not through its own copy, so a change in the extension shows in the apps and back.
2. **Extension-only choices are VS Code settings.** Which target and model the editor uses, the serve
   port, the `bragi` path, auto-start, inline and context options live in `settings.json`.
3. **Secrets never in `settings.json`.** Keys and tokens for things the extension owns (the serve
   token it imported, the direct endpoint key) are in the editor's `SecretStorage`. Keys for Bragi's
   own endpoints are sent once over the authenticated loopback API and kept by Bragi (OS keychain or
   sealed file); the extension only learns "key set".
4. **Don't duplicate parallel work.** MCP client servers are `feature/mcp-client` (LF-37). Serve
   auto-start, the restart command and the no-token field are `feature/serve-autostart` (LF-36). The
   extension builds on what those expose.

## Settings precedence

Workspace folder > workspace > user (global) > Bragi app defaults. VS Code resolves the first three
through `workspace.getConfiguration`. Keys marked `machine` or `application` scope are not read from
a workspace (a repository must not be able to point the extension at another program or port).
Anything that runs a program or opens a port (`bragi.path`, `bragi.serveUrl`, `bragi.serve.*`,
`bragi.mcp.servers` with commands) is `machine` scope, and untrusted workspaces are refused for them.

## Feature table

Legend: **done** = in the extension today or in this branch; **dep** = needs another branch; **left**.

| Feature | GPUI desktop | TUI | CLI (`bragi ...`) | Flutter mobile | Extension today | Extension plan |
|---|---|---|---|---|---|---|
| Link to an inference node (enter a link code) | yes | yes | `bragi link CODE` | yes | no | command + panel: enter code, compare the short code, confirm. Serve routes `/v1/link/*` (new here) |
| Link another device (show a code) | yes | yes | `bragi link` | yes | no | left: needs a QR/code view; same routes (`/v1/link/offer`) |
| List, edit, remove links | yes | yes | `bragi status` (list) | yes | no | Links view + commands. `/v1/links` (new here) |
| Add an inference node by address | yes | yes | `bragi add-node ADDR` | yes | no | command; `/v1/links` POST |
| OpenAI-compatible connections add / edit / remove / reorder / test | yes | yes | no | yes | one "direct" server (url, model, key in SecretStorage) | full list in the panel and commands; `/v1/endpoints` (new here). Keys go to Bragi's store, the direct server keeps SecretStorage |
| Fallback to a direct server | yes | yes | no | yes | `bragi.fallbackToDirect` | kept |
| Target and model selection | yes | yes | `--model` | yes | `Choose Target and Model`, status bar | kept; settings `bragi.target`, `bragi.model` get proper UI |
| Sessions and history (list, open, delete, folders) | yes | yes | no | yes | the chat keeps a conversation in Bragi's history, no list | left for after parity slice: list + open + delete from `/v1/sessions` (new here, list and delete only); folders left |
| Chat: text answer, stop, new | yes | yes | no | yes | yes | kept |
| Chat: attach files | yes | yes (picker) | `--context` | yes | editor context only | add "Attach file" and "Attach current selection" in the panel (left, after MCP) |
| Chat: images, audio | yes | no | no | yes | no | left: needs the serve chat route to take image and audio parts |
| Chat: reactions, delete a message | yes | yes | no | yes | no | left: needs serve routes |
| Themes (editor, switcher) | yes (LF-33) | yes (LF-33) | no | yes (LF-33) | follows the editor theme | no change: the extension uses the editor's colours; documented as intentional |
| `bragi serve` start / stop / status | yes (`--serve`) | yes (auto-start) | `bragi serve` | n/a | status only | **dep** LF-36 `ServeManager`; this branch adds Start / Stop / Show Status commands and the `bragi.serve.*` settings on top of it |
| `bragi mcp` server start / stop | n/a | n/a | `bragi mcp` | n/a | no | command that runs `bragi mcp` in a terminal (stdio is the client's), plus **Copy MCP Client Config** (Cursor, VS Code, Claude Desktop snippets) |
| MCP client servers (add, remove, enable, test, tool approval, "always allow") | yes (LF-37) | yes (LF-37) | n/a | yes (LF-37) | no | **dep** LF-37: `mcp-servers.json` and `McpServer` in `bragi-core`. Serve routes `/v1/mcp-servers` and the approval flow are added once that branch is in main; until then the panel shows a notice |
| Inline suggestions, rewrites, hunks | no | yes | `bragi code` | no | yes | kept |

## Settings contributed (package.json `contributes.configuration`)

Categories, in order, each a separate section so the UI groups them: **Bragi: Connection**,
**Bragi: Models**, **Bragi: Chat**, **Bragi: MCP**, **Bragi: Advanced**. Every key has a title,
description, `order`, `scope` and a default. Existing keys keep their names; only metadata is added.

| Key | Category | Scope |
|---|---|---|
| `bragi.target` | Models | window |
| `bragi.model`, `bragi.bragiTarget` | Models | resource |
| `bragi.direct.url`, `bragi.direct.model`, `bragi.fallbackToDirect` | Connection | machine |
| `bragi.serveUrl`, `bragi.tokenFile` | Connection | machine |
| `bragi.path`, `bragi.serve.autoStart` (LF-36), `bragi.serve.idleMinutes` | Advanced | machine |
| `bragi.inline.enabled`, `bragi.inline.debounceMs` | Chat | window |
| `bragi.context.openFiles`, `bragi.context.diagnostics` | Chat | resource |
| `bragi.mcp.server.args`, `bragi.mcp.server.allowWrites`, `bragi.mcp.server.allowDeletes` | MCP | machine |
| `bragi.mcp.clientServers` (JSON list, no secrets; schema) | MCP | machine |

Changing a setting takes effect without a reload (`onDidChangeConfiguration`).

## Serve API additions (this branch, `bragi-serve`)

All behind the same token and loopback guard as the rest of `/v1`. They call the `Core` methods the
apps already use; no new logic in core.

- `GET /v1/links`, `DELETE /v1/links/{address}`, `POST /v1/links/node` (add by address).
- `POST /v1/link/accept {code}` returns `{sas, device}`; `POST /v1/link/confirm {matches}`.
- `GET|POST /v1/endpoints`, `PUT|DELETE /v1/endpoints/{id}`, `POST /v1/endpoints/{id}/test`,
  `GET /v1/endpoints/{id}/models`. Responses are `EndpointView` (never a key).
- `GET /v1/sessions?offset&limit`, `DELETE /v1/sessions/{id}`.
- Later, with LF-37: `/v1/mcp-servers` (list, add, edit, remove, enable, test) and the approval stream.

## Tests

Extension: the headless `@vscode/test-electron` suite under Xvfb with `env -i` and a throwaway HOME
(never the live desktop): configuration scope metadata, precedence, command registration and behaviour
against the mock server. Serve: route tests in `crates/bragi-serve/tests`.
