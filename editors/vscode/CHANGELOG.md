# Changelog

## 0.2.0 (unreleased)

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
