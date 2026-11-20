// Rewrite the selection or the file: ask, show the change in a diff editor, then accept or reject.

import * as vscode from 'vscode';
import { Router } from './backends';
import { contextFor, contextOptions, isWholeFile } from './context';
import { BragiError, Scope } from './types';

export const PROPOSED_SCHEME = 'bragi-proposed';

interface Pending {
  original: vscode.Uri;
  /** The document's version when the rewrite was asked for. */
  version: number;
  range: vscode.Range;
  /** The text the rewrite replaces, to find it again if the document moved on. */
  target: string;
  code: string;
  proposed: vscode.Uri;
  /** The whole proposed file, shown on the right of the diff. */
  file: string;
}

export class Rewrites implements vscode.TextDocumentContentProvider {
  private readonly pending = new Map<string, Pending>();
  private readonly changed = new vscode.EventEmitter<vscode.Uri>();
  readonly onDidChange = this.changed.event;
  private next = 1;

  constructor(private readonly router: Router) {}

  provideTextDocumentContent(uri: vscode.Uri): string {
    return this.pending.get(uri.toString())?.file ?? '';
  }

  /** The pending rewrites (tests and the context key). */
  list(): readonly Pending[] {
    return [...this.pending.values()];
  }

  private async setContext(): Promise<void> {
    await vscode.commands.executeCommand('setContext', 'bragi.rewritePending', this.pending.size > 0);
  }

  /** Asks for a rewrite of `scope` in `editor` and opens the diff. `instruction` skips the prompt. */
  async start(editor: vscode.TextEditor, scope: Scope, instruction?: string): Promise<Pending | undefined> {
    const doc = editor.document;
    if (instruction === undefined) {
      instruction = await vscode.window.showInputBox({
        title: scope === 'file' ? 'Bragi: rewrite the file' : 'Bragi: rewrite the selection',
        prompt: 'How should the code change?',
        placeHolder: 'e.g. handle the error case, add doc comments, convert to async',
        ignoreFocusOut: true,
      });
    }
    if (!instruction?.trim()) {
      return undefined;
    }
    let range = scope === 'file' || editor.selection.isEmpty ? new vscode.Range(doc.positionAt(0), doc.positionAt(doc.getText().length)) : new vscode.Range(editor.selection.start, editor.selection.end);
    const ctx = contextFor(doc, range, contextOptions());
    if (!isWholeFile(doc, ctx)) {
      throw new BragiError('This file is too large to rewrite in one go; select the part to rewrite.', 'bad_request');
    }
    const version = doc.version;
    const result = await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title: 'Bragi is rewriting…', cancellable: true },
      async (_p, token) => {
        const abort = new AbortController();
        const sub = token.onCancellationRequested(() => abort.abort());
        try {
          const backend = await this.router.backend(abort.signal);
          return await backend.rewrite(ctx, instruction!, scope, this.router.modelFor(backend), abort.signal);
        } finally {
          sub.dispose();
        }
      },
    );
    if (result.scope === 'file') {
      range = new vscode.Range(doc.positionAt(0), doc.positionAt(doc.getText().length));
    }
    const target = doc.getText(range);
    const file = result.scope === 'file' ? result.code : ctx.before + result.code + ctx.after;
    const base = doc.uri.path.split('/').pop() || 'untitled';
    const proposed = vscode.Uri.from({ scheme: PROPOSED_SCHEME, path: `/${this.next++}/${base}`, query: doc.uri.toString() });
    const p: Pending = { original: doc.uri, version, range, target, code: result.code, proposed, file };
    // One pending rewrite per document: a new one replaces the old.
    for (const old of this.list().filter((x) => x.original.toString() === doc.uri.toString())) {
      this.pending.delete(old.proposed.toString());
    }
    this.pending.set(proposed.toString(), p);
    await this.setContext();
    if (target === result.code) {
      this.pending.delete(proposed.toString());
      await this.setContext();
      void vscode.window.showInformationMessage('Bragi suggested no change.');
      return undefined;
    }
    await vscode.commands.executeCommand('vscode.diff', doc.uri, proposed, `${base}: Bragi's rewrite (accept ✓ or reject ✕)`, { preview: true });
    return p;
  }

  /** The pending rewrite the user means: the one in the active diff, else the only one. */
  private chosen(uri?: vscode.Uri): Pending | undefined {
    const active = uri ?? vscode.window.activeTextEditor?.document.uri;
    if (active) {
      const direct = this.pending.get(active.toString());
      if (direct) {
        return direct;
      }
      const byOriginal = this.list().find((p) => p.original.toString() === active.toString());
      if (byOriginal) {
        return byOriginal;
      }
    }
    return this.pending.size === 1 ? this.list()[0] : undefined;
  }

  async accept(uri?: vscode.Uri): Promise<boolean> {
    const p = this.chosen(uri);
    if (!p) {
      void vscode.window.showWarningMessage('No Bragi rewrite is waiting.');
      return false;
    }
    const doc = await vscode.workspace.openTextDocument(p.original);
    let range = p.range;
    if (doc.version !== p.version || doc.getText(range) !== p.target) {
      // The document changed since: find the original text again, or give up.
      const at = doc.getText().indexOf(p.target);
      if (at < 0 || doc.getText().indexOf(p.target, at + 1) >= 0) {
        void vscode.window.showErrorMessage('The code changed since Bragi rewrote it; ask for the rewrite again.');
        return false;
      }
      range = new vscode.Range(doc.positionAt(at), doc.positionAt(at + p.target.length));
    }
    const edit = new vscode.WorkspaceEdit();
    edit.replace(p.original, range, p.code);
    const ok = await vscode.workspace.applyEdit(edit);
    if (ok) {
      await this.close(p);
    }
    return ok;
  }

  async reject(uri?: vscode.Uri): Promise<boolean> {
    const p = this.chosen(uri);
    if (!p) {
      return false;
    }
    await this.close(p);
    return true;
  }

  private async close(p: Pending): Promise<void> {
    this.pending.delete(p.proposed.toString());
    await this.setContext();
    const tabs = vscode.window.tabGroups.all
      .flatMap((g) => g.tabs)
      .filter((t) => t.input instanceof vscode.TabInputTextDiff && t.input.modified.toString() === p.proposed.toString());
    if (tabs.length) {
      await vscode.window.tabGroups.close(tabs);
    }
  }
}
