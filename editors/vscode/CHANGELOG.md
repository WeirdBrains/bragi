# Changelog

## 0.2.0 (unreleased)

- The rest of the apps' chat in the panel (LF-40): **File…** and **Selection** attach files, the selected
  text, images (PNG, JPEG, GIF, WebP) and sound (WAV, MP3) to the next message (also **Attach File to
  Chat…** in the Explorer menu, **Attach Selection to Chat** in the editor menu, or a drop); images show and
  sound plays in the message. Under each message: quick reactions and delete (a question can take its answer
  with it; **Undo** puts it back). **Open Chat from History…** loads a chat, grouped by folder, and the next
  message continues it; **New / Rename / Move / Delete Chat Folder…** and **Move Chat to Folder…** file
  chats. **Link This Bragi to Another Device (Show a Code)…** shows a link code (copied), then the short code
  to compare. Images and sound need the Bragi target; the direct target gets text files inline.
- Agent mode and outside MCP servers in full (LF-40 on LF-37): the chat panel's **Agent** switch lets the
  model use tools (a node's, or the outside MCP servers that apply on an endpoint); each call shows a card
  with **Allow once**, **Allow for this task**, **Always allow** (outside MCP servers' tools) and **Deny**.
  Adding a server asks where its tools may be used (every chat, or code chats in this folder); servers can be
  edited (the stored token stays unless you type a new one) and tested, and **Take Back "Always Allow" for an
  MCP Tool…** makes a tool ask again. Needs `bragi serve` with `/v1/mcp-servers` and `/v1/agent/approve`.
- Video in the chat (LF-43, V2): attach a clip with **Video…**, **Bragi: Attach Video to Chat…**, the
  Explorer's context menu or a drop on the panel; Bragi samples it for the model (the extension runs
  no ffmpeg). A chip shows the model's video limits and **Limits…** / **Bragi: Set Video Limits for
  the Model…** sets this device's override. The chat plays the clip in the panel with the frames the
  model saw beneath it, or says why it can't and offers **Open in system player**. The webview reads
  only the folders of clips it plays. Bragi target only; the direct target refuses a clip.
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
