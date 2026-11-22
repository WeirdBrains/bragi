# MCP client: adding outside MCP servers to Bragi and Ymir

Status: design, 2026-11-22 (LF-37). Both apps can already *be* MCP servers (`bragi mcp`, `ymirctl mcp`).
This adds the other direction: the user adds outside MCP servers (stdio and streamable HTTP) and their
tools are offered to models through tool calling, with the tool calls run and the results fed back.

## What exists today

| Piece | State |
|---|---|
| `weirdnet-mcp` 0.6.1, feature `client` | Usable. `Client::connect` (initialize), `list_tools` (paged), `call_tool` and `call_tool_until` (timeout and cancel, sends `notifications/cancelled`), `ping`, `close`. Transports: `StdioTransport` (local command, env, cwd, 4 MiB line cap) and `HttpTransport` (streamable HTTP, JSON or SSE, bearer and custom headers). `Secret` is redacted in `Debug`. `StdioCommand::warning()` gives the "runs as you" text. |
| `weirdnet-mcp` feature `agent` | `McpSource` is a `weirdnet_agent::ToolSource`: annotations map to tiers (floor `Research`), name prefix, output cap. Works with weirdnet-agent 0.5.0 (`>=0.4, <0.6`). |
| `weirdnet-agent` 0.5.0 | `AgentSession` is the tool-call loop (`Budget::max_steps`, cancel, streaming events), `ToolRegistry` (enable per source or tool, tier overrides), `Policy` with approvals and `StandingAllow` ("always allow"), `Approver`, and remote tasks (`/weirdnet-agent/task/1`) that forward approval cards to the asking device. `infer/2` carries `tools` and `tool_calls`. |
| `weirdnet-openai` 0.2.0 | Maps OpenAI `tools`, assistant `tool_calls` and `tool` messages to the agent types; `weirdnet-backends` joins streamed tool-call fragments (`ToolCalls`). |
| Ymir | The node already runs agent tasks (`ymir-core/src/tasks.rs`) with one tool, `files.read`, and approvals asked on the asking device. Tools and tool calls pass through the coordinator, the bridge's OpenAI endpoint and llama.cpp (started with `--jinja`). **No outside MCP tools.** |
| Bragi | Agent mode (tool calls with approval cards) runs on a Ymir node's task runtime. `Core::complete_to` and the plain chat path send no `tools`. **No MCP client, no tools for OpenAI endpoints.** |
| Models | Tool calling works when the llama.cpp build and the model's jinja chat template support it. Ymir starts llama.cpp with `--jinja`; whether a given model calls tools well is not recorded in the catalog. |

Conclusion: no blocker in weirdnet-*. The client, the `ToolSource` adapter and the loop exist. The wish list
below is for robustness and shared config, not for basics.

## Where servers are configured

**Ymir (server side).** `node.toml`, per node, under the agent section: repeated `[[inference.agent.mcp]]`
entries with `id`, `enabled`, `transport = "stdio" | "http"`, then for stdio `command`, `args`, `cwd`, `env`
(name to secret source), and for http `url`, `bearer`, `headers` (name to secret source). Secret sources are
`env:NAME` or `file:/path`, like `wrap_api_key`; a literal secret is refused by validation. Managed with
`ymirctl mcp-servers add | list | remove | enable | disable | test`. A pool-wide default is a later
step: the pool has no shared config store, so each node lists its own.

**Bragi (client side).** Settings has an "MCP servers" section on the desktop, Flutter and terminal apps:
add, edit, remove, enable, test. Each server has a scope: all chats (default), one folder (code mode), or one
session. Config (no secrets) lives with the other settings; env values and header or bearer values live in the
secret store next to API keys (OS keychain when opted in, otherwise the sealed file), shown as "set" only.

## Which side runs tools

- **Ymir runs the tools** for tasks served by that node (agent mode from Bragi, `ymirctl chat --tools` later).
  The node's servers, budget and policy apply; approval cards go to the asking device.
- **Bragi runs the tools** when the target is an OpenAI-compatible endpoint (no Ymir task runtime): bragi-core
  builds a local `AgentSession` with a `ModelBackend` over the endpoint and `McpSource`s for the configured servers.
- With the pool as target, Bragi sends the task to a node; the node's servers are used, and Bragi's own are
  not forwarded (a node should not run commands a phone defined). Bragi's settings show which node's tools
  are in play.

## How tools reach models, and the loop

1. On connect (and on `tools/list_changed` or manual refresh) each enabled server is listed and its tools are
   registered as `<server-id>.<tool>` in the `ToolRegistry`, so names stay unique and the user sees the origin.
2. `ToolRegistry::specs()` becomes the `tools` array of the request (OpenAI function tools).
3. `AgentSession` runs the loop: model turn, if it has `tool_calls` then policy and approval, then
   `call_tool_until` with a timeout and the task's cancel, then a `tool` message with the result wrapped as
   untrusted (the crate already marks it `untrusted_tool_result`), then the next turn.
4. The loop ends on an answer, an error, cancel or `Budget::max_steps` (default 8 for MCP tasks; a setting per
   app). Output over the cap is cut with a note.

## Approval UX

Every call from an outside server asks first: server, tool, the arguments as JSON, the tier, and for stdio
the command. Answers: allow once, always allow this tool (a `StandingAllow`, revocable in Settings or
`ymirctl mcp-servers allow --revoke`), or deny. Read-only closed-world tools may be offered "always allow"
first; destructive or undeclared tools never get a blanket "allow all". A server's annotations are hints only
and are floored at `Research`. Servers added on a node can only be allowed from a linked device.

## Secrets

Env values, bearer tokens and header values are `weirdnet_mcp::client::Secret`s: never printed, never in
`node.toml`, settings JSON, logs, traces or session history. Tool arguments and results do go in the session
history (as agent mode's tool names do today); results are not logged by the node.

## Security

- A stdio server runs as the user (Bragi) or as the `ymird` service user (Ymir) with that user's permissions.
  Adding one shows `StdioCommand::warning()` and needs a confirm; on a node it needs the operator at
  `ymirctl`, never a remote request.
- HTTP servers must be `https://`, except loopback; no redirects to plain http.
- Tools are never exposed over Weirdnet except through the existing task protocol, which already checks
  the device roster (`RosterAuth`). No new protocol, no unauthenticated path.
- Tool output is untrusted text (prompt injection): it is wrapped, capped, and cannot change policy.
- Per-call timeout (60 s default) and per-server concurrency cap; a crashing stdio server is restarted with
  backoff and its calls fail cleanly.
- Only `files.read`-style workspace limits apply to Ymir's own tool; MCP servers are not sandboxed by Ymir.

## What weirdnet-mcp should add (wish list for the Weirdnet Lead)

Generic pieces, so neither app writes them:

1. **A shared server config type** (`client::config`, serde): `ServerConfig { id, enabled, transport, command, args,
   cwd, env, url, bearer, headers, tier_floor, timeout }`, secret fields as `env:` / `file:` / opaque `SecretRef`,
   `validate()` (https unless loopback, no literal secrets in file form), and `connect(&resolver) -> McpSource`.
2. **A server manager**: `ServerSet` that connects many, reports per-server status (connecting, ready, failed
   with reason, tool count), reconnects stdio with backoff, and re-lists on `notifications/tools/list_changed`.
3. **Client-side `resources/list`, `resources/read`, `resources/templates/list`** and `prompts/list`, `prompts/get`
   (the client has tools only).
4. **OAuth for HTTP servers** (MCP authorization: discovery, PKCE, refresh) with a token store trait the app
   backs with its keychain.
5. **Progress and cancellation hooks**: `progressToken` support so a long call reports progress to the UI.
6. **`McpSource` rename and output controls**: per-server `allow`/`deny` tool lists, and an option to
   expose the server's `title`/`annotations` to the approval prompt without copying them by hand.
7. **Test helper**: an in-process server and client pair over `StdioTransport::from_io` for app tests.
8. **An OpenAI tool-schema helper**: `RemoteTool -> function tool JSON`, with schema clean-up (drop `$schema`,
   unsupported keywords) for llama.cpp's grammar-based tool calling.
9. **weirdnet-agent**: a standing-allow store trait the app persists (the allows must survive restarts), and an
   `AgentSession` entry that takes a list of `ToolSource`s plus an endpoint `ModelBackend` for the Bragi
   local-loop case (a `weirdnet-backends` OpenAI-endpoint `ModelBackend`).

## Build plan

1. Ymir (`feature/mcp-client`): `AgentMcp` config and validation, `tasks::serve` adds `McpSource`s, `ymirctl
   mcp-servers` commands, tests against Ymir's own MCP server as the demo server.
2. Bragi (`feature/mcp-client`): config and secret storage in bragi-core, the local loop behind a trait, Settings
   screens on all three apps, tests against `bragi mcp`.
3. Docs on the site, and a real-client pass (a real stdio server with a real model on a node).
