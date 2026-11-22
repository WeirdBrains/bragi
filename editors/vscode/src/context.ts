// Gathers what the editor shows into an EditorContext.

import * as vscode from 'vscode';
import { Diagnostic, EditorContext, OpenFile } from './types';

/** Caps so a huge file doesn't become a huge request; bragi-core trims further. */
const MAX_BEFORE = 400_000;
const MAX_AFTER = 200_000;
const MAX_OPEN_FILE = 8_000;
const MAX_OPEN_FILES = 3;
const MAX_DIAGNOSTICS = 40;

export interface ContextOptions {
  openFiles: boolean;
  diagnostics: boolean;
}

export function contextOptions(): ContextOptions {
  const c = vscode.workspace.getConfiguration('bragi');
  return { openFiles: c.get('context.openFiles', true), diagnostics: c.get('context.diagnostics', true) };
}

export function relativePath(uri: vscode.Uri): string {
  if (uri.scheme === 'untitled') {
    return uri.path;
  }
  return vscode.workspace.asRelativePath(uri, false);
}

const severities: Record<number, Diagnostic['severity']> = {
  [vscode.DiagnosticSeverity.Error]: 'error',
  [vscode.DiagnosticSeverity.Warning]: 'warning',
  [vscode.DiagnosticSeverity.Information]: 'info',
  [vscode.DiagnosticSeverity.Hint]: 'hint',
};

/** The file's problems, nearest the cursor first. */
export function diagnosticsFor(doc: vscode.TextDocument, near: vscode.Position): Diagnostic[] {
  return vscode.languages
    .getDiagnostics(doc.uri)
    .slice()
    .sort((a, b) => a.severity - b.severity || Math.abs(a.range.start.line - near.line) - Math.abs(b.range.start.line - near.line))
    .slice(0, MAX_DIAGNOSTICS)
    .map((d) => ({
      line: d.range.start.line + 1,
      severity: severities[d.severity] ?? 'info',
      message: d.message,
      source: d.source,
    }));
}

/** Other text documents open in editor tabs (only documents the editor already loaded). */
export function openFiles(current: vscode.TextDocument): OpenFile[] {
  const loaded = new Map(vscode.workspace.textDocuments.map((d) => [d.uri.toString(), d]));
  const out: OpenFile[] = [];
  const seen = new Set([current.uri.toString()]);
  for (const group of vscode.window.tabGroups.all) {
    for (const tab of group.tabs) {
      const input = tab.input;
      if (!(input instanceof vscode.TabInputText)) {
        continue;
      }
      const key = input.uri.toString();
      const doc = loaded.get(key);
      if (!doc || seen.has(key) || doc.uri.scheme === 'bragi-proposed') {
        continue;
      }
      seen.add(key);
      out.push({ path: relativePath(doc.uri), language: doc.languageId, text: doc.getText().slice(0, MAX_OPEN_FILE) });
      if (out.length >= MAX_OPEN_FILES) {
        return out;
      }
    }
  }
  return out;
}

/** The context around `range` (the selection) in `doc`. */
export function contextFor(doc: vscode.TextDocument, range: vscode.Range, opts: ContextOptions): EditorContext {
  const text = doc.getText();
  const start = doc.offsetAt(range.start);
  const end = doc.offsetAt(range.end);
  return {
    path: relativePath(doc.uri),
    language: doc.languageId,
    before: text.slice(Math.max(0, start - MAX_BEFORE), start),
    selection: text.slice(start, end),
    after: text.slice(end, end + MAX_AFTER),
    open_files: opts.openFiles ? openFiles(doc) : [],
    diagnostics: opts.diagnostics ? diagnosticsFor(doc, range.end) : [],
  };
}

/** True when `ctx` holds the whole document (nothing was cut off at either end). */
export function isWholeFile(doc: vscode.TextDocument, ctx: EditorContext): boolean {
  return ctx.before.length + ctx.selection.length + ctx.after.length === doc.getText().length;
}

export function editorContext(editor: vscode.TextEditor, opts = contextOptions()): EditorContext {
  return contextFor(editor.document, new vscode.Range(editor.selection.start, editor.selection.end), opts);
}
