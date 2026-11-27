# Extension parity: the VS Code and Cursor extension does what Bragi does

Status: done, 2026-11-26 (LF-40; designed 2026-11-22). Owner: [m3talsmith](https://github.com/m3talsmith).

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
| Link another device (show a code) | yes | yes | `bragi link` | yes | **done** (2026-11-26) | **Link This Bragi to Another Device (Show a Code)…**: the code (copied) in a notification, the short code to compare, confirm. `/v1/link/offer`, `/offer/confirm`, `/offer/cancel`; progress is `pair` in `/v1/state` |
| List, edit, remove links | yes | yes | `bragi status` (list) | yes | no | Links view + commands. `/v1/links` (new here) |
| Add an inference node by address | yes | yes | `bragi add-node ADDR` | yes | no | command; `/v1/links` POST |
| OpenAI-compatible connections add / edit / remove / reorder / test | yes | yes | no | yes | one "direct" server (url, model, key in SecretStorage) | full list in the panel and commands; `/v1/endpoints` (new here). Keys go to Bragi's store, the direct server keeps SecretStorage |
| Fallback to a direct server | yes | yes | no | yes | `bragi.fallbackToDirect` | kept |
| Target and model selection | yes | yes | `--model` | yes | `Choose Target and Model`, status bar | kept; settings `bragi.target`, `bragi.model` get proper UI |
| Sessions and history (list, open, delete, folders) | yes | yes | no | yes | **done** (2026-11-26) | **Open Chat from History…** (grouped by folder) loads a chat into the panel; **Delete Chats…**; **New / Rename / Move / Delete Chat Folder…** and **Move Chat to Folder…**. `GET /v1/sessions/{id}`, `/v1/folders`, `/v1/sessions/{id}/folder` |
| Chat: text answer, stop, new | yes | yes | no | yes | yes | kept |
| Chat: attach files | yes | yes (picker) | `--context` | yes | **done** (2026-11-26) | **File…** and **Selection** in the panel, **Attach File to Chat…** (Explorer menu), **Attach Selection to Chat** (editor menu), or a drop; chips show what goes with the next message. `/v1/code/chat` `attachments` |
| Chat: images, audio | yes | no | no | yes | **done** (2026-11-26) | PNG, JPEG, GIF, WebP and WAV, MP3 through the same attach; shown and playable in the message (and in an opened chat). Bragi target only; the direct target refuses them |
| Chat: reactions, delete a message | yes | yes | no | yes | **done** (2026-11-26) | quick reactions and delete under each message once Bragi has its id; deleting a question offers its answer too, with **Undo**. `/v1/sessions/{id}/messages/{m}/react`, `DELETE …/messages/{m}`, `/v1/undo` |
| Themes (editor, switcher) | yes (LF-33) | yes (LF-33) | no | yes (LF-33) | follows the editor theme | no change: the extension uses the editor's colours; documented as intentional |
| `bragi serve` start / stop / status | yes (`--serve`) | yes (auto-start) | `bragi serve` | n/a | status only | **dep** LF-36 `ServeManager`; this branch adds Start / Stop / Show Status commands and the `bragi.serve.*` settings on top of it |
| `bragi mcp` server start / stop | n/a | n/a | `bragi mcp` | n/a | no | command that runs `bragi mcp` in a terminal (stdio is the client's), plus **Copy MCP Client Config** (Cursor, VS Code, Claude Desktop snippets) |
| MCP client servers (add, remove, enable, test, tool approval, "always allow") | yes (LF-37) | yes (LF-37) | n/a | yes (LF-37) | **done** (2026-11-26) | `/v1/mcp-servers` routes and `/v1/agent/approve`; the Connections view and commands (scope, edit, test, take back "always allow"); the chat's **Agent** switch and approval cards (allow once, for this task, always, deny) |
| Inline suggestions, rewrites, hunks | no | yes | `bragi code` | no | yes | kept |

## Settings contributed (package.json `contributes.configuration`)

Categories, in order, each a separate section so the UI groups them: **Bragi: Connection**,
**Bragi: Models**, **Bragi: Chat**, **Bragi: MCP**, **Bragi: Advanced**. Every key has a title,
description, `order`, `scope` and a default. Existing keys keep their names; only metadata is added.

| Key | Category | Scope |
|---|---|---|
| `bragi.target` | Models | window |
| `bragi.model`, `bragi.bragiTarget`, `bragi.direct.model` | Models | window |
| `bragi.direct.url` | Connection | machine |
| `bragi.fallbackToDirect` | Connection | window |
| `bragi.serveUrl`, `bragi.tokenFile` | Connection | machine |
| `bragi.path` | Advanced | machine |
| `bragi.autoStartServer` (LF-36) | Connection | machine |
| `bragi.inline.enabled`, `bragi.inline.debounceMs` | Chat | window |
| `bragi.context.openFiles`, `bragi.context.diagnostics` | Chat | resource |
| `bragi.mcp.server.args`, `.allowWrites`, `.allowDeletes` | MCP | machine |
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
- With LF-37 (built 2026-11-26, `feature/ext-parity-mcp`): `GET|POST /v1/mcp-servers`, `PUT|DELETE /v1/mcp-servers/{id}`,
  `POST /v1/mcp-servers/{id}/enabled {enabled}`, `…/{id}/test`, `…/{id}/always-allow {tool, allow}`, responses
  `McpServerView` (never a secret; `secrets {env, headers, bearer}` taken on add and edit, an edit without them
  keeps what is stored; `scope` is `"all"`, `"folder"` with `folder`, or `{kind, value}`). `/v1/code/chat` takes
  `agent` (a task on a node for the pool, Bragi's own tool-call loop with the MCP servers that apply for an
  endpoint) and `folder`; its `approval` events carry `always` (an outside MCP server's tool) and are answered with
  `POST /v1/agent/approve {request_id, ask, choice}` (`deny`, `once`, `this_task`, `always`).

## Status (2026-11-22)

Done on `feature/extension-parity`: the settings sections and scopes, the serve management routes
(links, endpoints, sessions), the commands and the Connections view for links, OpenAI connections,
chats, server stop and status, `bragi mcp` start, stop and copy config, and the MCP client server
commands with `bragi.mcp.clientServers`.

MCP client servers and approvals (2026-11-26, `feature/ext-parity-mcp`): the `/v1/mcp-servers` routes and
`/v1/agent/approve` in `bragi serve`; in the extension, the server's scope (every chat, or code chats in this
folder) when adding, edit, test, **Take Back "Always Allow" for an MCP Tool…**, and the chat panel's **Agent**
switch: tool calls show a card (the tool, its tier and site, what it will do) with **Allow once**, **Allow for
this task**, **Always allow** (outside MCP servers' tools only) and **Deny**; tool runs and notices show in the
answer, and a card left open when the answer ends says so. An older Bragi without the routes still says it
can't manage MCP servers.

The rest (2026-11-26, `feature/ext-parity-chat`): files, the selection, images and sound attached in the
chat panel (`/v1/code/chat` takes `attachments: [{name, text} | {name, data}]`, checked by bragi-core's
`Attachment::from_bytes`; the route takes up to 48 MiB); reactions and deleting a message with undo; opening
a chat from history into the panel; chat folders; and showing a link code for another device. Serve routes
in `crates/bragi-serve/src/chats.rs`:

- `GET /v1/sessions/{id}`: `{conversation, folder, target, unbound}`.
- `POST /v1/sessions/{id}/messages/{message}/react {emoji}` → `{reactions}`;
  `DELETE /v1/sessions/{id}/messages/{message}?with_reply=true` → `{undo, messages, ids}`; `POST /v1/undo {undo}`.
- `GET|POST /v1/folders`, `POST /v1/folders/{id}/rename {name}`, `/move {parent}`, `DELETE /v1/folders/{id}`,
  `POST /v1/sessions/{id}/folder {folder}`.
- `POST /v1/link/offer` → `{link, expires_ms}`, `POST /v1/link/offer/confirm {matches}` (409 with nothing to
  confirm), `POST /v1/link/offer/cancel`; `GET /v1/state` has `pair`.

Nothing in this document is left open.

## Tests

Extension: the headless `@vscode/test-electron` suite under Xvfb with `env -i` and a throwaway HOME
(never the live desktop): configuration scope metadata, precedence, command registration and behaviour
against the mock server. Serve: route tests in `crates/bragi-serve/tests`.
