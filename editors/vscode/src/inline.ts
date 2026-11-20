// Inline suggestions as you type, from the chosen backend.

import * as vscode from 'vscode';
import { Router } from './backends';
import { contextFor, contextOptions } from './context';
import { BragiError } from './types';

function sleep(ms: number, token: vscode.CancellationToken): Promise<boolean> {
  return new Promise((resolve) => {
    if (ms <= 0) {
      resolve(!token.isCancellationRequested);
      return;
    }
    const t = setTimeout(() => {
      sub.dispose();
      resolve(!token.isCancellationRequested);
    }, ms);
    const sub = token.onCancellationRequested(() => {
      clearTimeout(t);
      sub.dispose();
      resolve(false);
    });
  });
}

export class InlineProvider implements vscode.InlineCompletionItemProvider {
  /** The last answer, for the same spot asked again (VS Code re-asks often). */
  private last?: { key: string; text: string };
  /** Shown once per problem, not on every keystroke. */
  private lastError?: string;
  readonly onError = new vscode.EventEmitter<BragiError>();

  constructor(private readonly router: Router) {}

  async provideInlineCompletionItems(
    doc: vscode.TextDocument,
    position: vscode.Position,
    _context: vscode.InlineCompletionContext,
    token: vscode.CancellationToken,
  ): Promise<vscode.InlineCompletionItem[]> {
    const cfg = vscode.workspace.getConfiguration('bragi');
    if (!cfg.get('inline.enabled', true) || !vscode.workspace.isTrusted || doc.uri.scheme === 'bragi-proposed') {
      return [];
    }
    const key = `${doc.uri.toString()}@${doc.version}:${doc.offsetAt(position)}`;
    if (this.last?.key === key) {
      return this.items(this.last.text, position);
    }
    if (!(await sleep(cfg.get('inline.debounceMs', 350), token))) {
      return [];
    }
    const abort = new AbortController();
    const sub = token.onCancellationRequested(() => abort.abort());
    try {
      const backend = await this.router.backend(abort.signal);
      const ctx = contextFor(doc, new vscode.Range(position, position), { ...contextOptions(), openFiles: false });
      const text = await backend.suggest(ctx, this.router.modelFor(backend), abort.signal);
      if (token.isCancellationRequested) {
        return [];
      }
      this.last = { key, text };
      this.lastError = undefined;
      return this.items(text, position);
    } catch (e) {
      const err = e instanceof BragiError ? e : new BragiError(String(e), 'other');
      if (err.kind !== 'cancelled' && !token.isCancellationRequested && err.message !== this.lastError) {
        this.lastError = err.message;
        this.onError.fire(err);
      }
      return [];
    } finally {
      sub.dispose();
    }
  }

  private items(text: string, position: vscode.Position): vscode.InlineCompletionItem[] {
    return text ? [new vscode.InlineCompletionItem(text, new vscode.Range(position, position))] : [];
  }
}
