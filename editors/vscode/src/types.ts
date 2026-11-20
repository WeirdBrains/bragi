// The editor context and answers, matching bragi-core's `code` module and `bragi serve`'s JSON.

export interface OpenFile {
  path: string;
  language: string;
  text: string;
}

export interface Diagnostic {
  /** 1-based. */
  line: number;
  severity: 'error' | 'warning' | 'info' | 'hint';
  message: string;
  source?: string;
}

/** `before + selection + after` is the file; an empty selection is the cursor. */
export interface EditorContext {
  path: string;
  language: string;
  before: string;
  selection: string;
  after: string;
  open_files: OpenFile[];
  diagnostics: Diagnostic[];
}

export type Scope = 'selection' | 'file';

export interface ModelInfo {
  id: string;
  status?: string;
}

export interface ModelList {
  models: ModelInfo[];
  defaultModel?: string;
  /** Why there is nothing to answer with, when there isn't. */
  message?: string;
}

export interface RewriteResult {
  /** What replaces the target (the selection, or the whole file). */
  code: string;
  scope: Scope;
  model?: string;
}

/** What a chat stream says, mirroring bragi-core's ChatEvent (the subset the panel draws). */
export type ChatEvent =
  | { type: 'conversation'; id: string; request_id: string }
  | { type: 'started'; node: string; model: string }
  | { type: 'delta'; offset: number; text: string }
  | { type: 'reasoning'; offset: number; text: string }
  | { type: 'queued'; position: number }
  | { type: 'retry'; attempt: number; message: string }
  | { type: 'done'; finish_reason?: string }
  | { type: 'error'; kind: string; message: string; partial?: string }
  | { type: string; [k: string]: unknown };

export class BragiError extends Error {
  constructor(
    message: string,
    readonly kind: string,
    readonly status?: number,
  ) {
    super(message);
  }
}
