# Changelog

## 0.2.0 (unreleased)

- The extension can set Bragi up, not just use it (LF-40). New commands and a **Connections** view in
  the Bragi activity bar: link to an inference node (enter a link code, compare the short code,
  confirm) or add one by address; add, edit, test and remove OpenAI-compatible connections (keys go
  to Bragi's secret store once and are never shown or kept in the editor); delete chats; stop the
  local server and show its state; start `bragi mcp` in a terminal and copy its config for Cursor,
  VS Code or Claude Desktop; add, remove, turn off and test outside MCP servers (needs a Bragi with
  the MCP client; older ones say so).
- Settings are grouped in the Settings UI as **Bragi: Connection, Models, Chat, MCP, Advanced**, each with a
  title, description and scope. User and workspace `settings.json` are both honoured (workspace
  folder, then workspace, then user, then Bragi's defaults); keys that run a program or point at a
  server are machine scope, so a repository can't set them. **Bragi: Show Settings and Where They
  Come From** lists each value and the scope it came from. New keys: `bragi.mcp.server.allowWrites`,
  `bragi.mcp.server.allowDeletes`, `bragi.mcp.server.args`, `bragi.mcp.clientServers`.
- No API token yet no longer shows a warning in the target picker: the token in the Bragi home is read silently, and otherwise a field asks for it (kept in secret storage, never shown), with a button to import it from the Bragi app.
- Bragi's server starts by itself: when nothing answers, the extension runs `bragi serve` in the
  background (`bragi.autoStartServer`, `bragi.path`), shows its state in the status bar and has a
  **Bragi: Start/Restart Server** command. It never stops a server it didn't start, and one it
  starts stops after 30 idle minutes and starts again on the next request.
- Rewrites are reviewed hunk by hunk: Accept and Reject over each hunk, Accept All and Reject
  All, the editor's undo for an accepted hunk, and the hunks from `bragi serve`
  (`/v1/code/hunks`, `/v1/code/apply`), the same as the terminal app's.
- First version: inline suggestions, rewrite selection or file with a diff to accept or reject,
  a chat panel with editor context, and a target and model picker, through `bragi serve` or a
  direct OpenAI-compatible server.
