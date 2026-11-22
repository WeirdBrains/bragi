# Changelog

## 0.2.0 (unreleased)

- Rewrites are reviewed hunk by hunk: Accept and Reject over each hunk, Accept All and Reject
  All, the editor's undo for an accepted hunk, and the hunks from `bragi serve`
  (`/v1/code/hunks`, `/v1/code/apply`), the same as the terminal app's.
- First version: inline suggestions, rewrite selection or file with a diff to accept or reject,
  a chat panel with editor context, and a target and model picker, through `bragi serve` or a
  direct OpenAI-compatible server.
