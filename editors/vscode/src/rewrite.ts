// Rewrite the selection or the file, then review it hunk by hunk: each hunk gets Accept and
// Reject CodeLenses in the editor (and its removed lines are marked), the whole proposal is in a
// diff beside it, and Accept All / Reject All finish it at once. Accepting edits the document,
// so the editor's own undo takes it back (and the hunk comes back for review).
//
// The hunks come from the backend: bragi-core's `code::FileDiff` through `bragi serve`, the same
// review the terminal app does (worked out locally only for the direct target).

import * as vscode from 'vscode';
import { Backend, Router } from './backends';
import { contextFor, contextOptions, isWholeFile } from './context';
import { anchorLine, countLines, Hunk, hunkText, mapOffset, minimalEdit } from './hunks';
import { BragiError, Scope } from './types';

export const PROPOSED_SCHEME = 'bragi-proposed';

export interface Pending {
  original: vscode.Uri;
  /** The proposal, shown on the right of the diff. */
  proposed: vscode.Uri;
  /** What Bragi answered to replace the target (the selection or the file). */
  code: string;
  /** The whole proposed file: where every hunk not rejected yet leads. */
  file: string;
  /** The hunks still to review, from the document to `file`. */
  hunks: Hunk[];
  /** Bumped whenever the hunks change, so an old CodeLens does nothing. */
  generation: number;
}

interface Review extends Pending {
  backend: Backend;
  /** The document as the review last saw it (changes are worked through in order). */
  shadow: string;
  shadowVersion: number;
  /** The version our next edit will give the document. */
  expectVersion?: number;
  /** Our accepted edits, to tell their undo and redo from the user's own edits. */
  ours: { removed: string; inserted: string }[];
  /** Every hunk is in: kept a little while so undo can bring them back. */
  resolved: boolean;
  queue: Promise<unknown>;
}

const removedLine = vscode.window.createTextEditorDecorationType({
  isWholeLine: true,
  backgroundColor: new vscode.ThemeColor('diffEditor.removedLineBackground'),
  overviewRulerColor: new vscode.ThemeColor('editorOverviewRuler.deletedForeground'),
  overviewRulerLane: vscode.OverviewRulerLane.Left,
});
const addedHint = vscode.window.createTextEditorDecorationType({
  after: { color: new vscode.ThemeColor('editorCodeLens.foreground'), fontStyle: 'italic', margin: '0 0 0 2em' },
});

export class Rewrites implements vscode.TextDocumentContentProvider, vscode.CodeLensProvider {
  private readonly reviews = new Map<string, Review>();
  private readonly changed = new vscode.EventEmitter<vscode.Uri>();
  readonly onDidChange = this.changed.event;
  private readonly lensesChanged = new vscode.EventEmitter<void>();
  readonly onDidChangeCodeLenses = this.lensesChanged.event;
  private next = 1;

  constructor(private readonly router: Router) {}

  /** Follows the documents and editors under review. */
  register(): vscode.Disposable {
    return vscode.Disposable.from(
      this.changed,
      this.lensesChanged,
      vscode.workspace.registerTextDocumentContentProvider(PROPOSED_SCHEME, this),
      vscode.languages.registerCodeLensProvider([{ pattern: '**' }, { scheme: PROPOSED_SCHEME }], this),
      vscode.workspace.onDidChangeTextDocument((e) => this.onEdit(e)),
      vscode.workspace.onDidCloseTextDocument((d) => {
        const r = this.reviews.get(d.uri.toString());
        if (r) {
          void this.drop(r);
        }
      }),
      vscode.window.onDidChangeVisibleTextEditors(() => this.decorate()),
    );
  }

  provideTextDocumentContent(uri: vscode.Uri): string {
    return this.byProposed(uri)?.file ?? '';
  }

  /** The rewrites waiting for review (tests and the context key). */
  list(): readonly Pending[] {
    return [...this.reviews.values()].filter((r) => !r.resolved);
  }

  /** Resolves when every change and action on `uri`'s review so far has been worked through. */
  async settled(uri?: vscode.Uri): Promise<void> {
    for (const r of uri ? [this.find(uri)].filter((x): x is Review => !!x) : [...this.reviews.values()]) {
      let q: Promise<unknown>;
      do {
        q = r.queue;
        await q;
      } while (q !== r.queue);
    }
  }

  private byProposed(uri: vscode.Uri): Review | undefined {
    return [...this.reviews.values()].find((r) => r.proposed.toString() === uri.toString());
  }

  private find(uri: vscode.Uri): Review | undefined {
    return this.reviews.get(uri.toString()) ?? this.byProposed(uri);
  }

  private async setContext(): Promise<void> {
    await vscode.commands.executeCommand('setContext', 'bragi.rewritePending', this.list().length > 0);
  }

  /** Runs `job` after everything already queued on `r`. */
  private enqueue<T>(r: Review, job: () => Promise<T>): Promise<T> {
    const run = r.queue.then(job, job);
    r.queue = run.catch(() => undefined);
    return run;
  }

  /** Asks for a rewrite of `scope` in `editor` and opens the review. `instruction` skips the prompt. */
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
    const range = scope === 'file' || editor.selection.isEmpty ? new vscode.Range(doc.positionAt(0), doc.positionAt(doc.getText().length)) : new vscode.Range(editor.selection.start, editor.selection.end);
    const ctx = contextFor(doc, range, contextOptions());
    if (!isWholeFile(doc, ctx)) {
      throw new BragiError('This file is too large to rewrite in one go; select the part to rewrite.', 'bad_request');
    }
    const asked = ctx.before + ctx.selection + ctx.after;
    const { backend, result } = await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title: 'Bragi is rewriting…', cancellable: true },
      async (_p, token) => {
        const abort = new AbortController();
        const sub = token.onCancellationRequested(() => abort.abort());
        try {
          const backend = await this.router.backend(abort.signal);
          return { backend, result: await backend.rewrite(ctx, instruction!, scope, this.router.modelFor(backend), abort.signal) };
        } finally {
          sub.dispose();
        }
      },
    );
    let file = result.scope === 'file' ? result.code : ctx.before + result.code + ctx.after;
    const now = doc.getText();
    if (now !== asked) {
      // The document moved on while Bragi worked: put the rewrite where its target is now.
      const target = result.scope === 'file' ? asked : ctx.selection;
      const at = now.indexOf(target);
      if (result.scope === 'file' || at < 0 || now.indexOf(target, at + 1) >= 0) {
        throw new BragiError('The code changed while Bragi rewrote it; ask for the rewrite again.', 'conflict');
      }
      file = now.slice(0, at) + result.code + now.slice(at + target.length);
    }
    if (file === now) {
      void vscode.window.showInformationMessage('Bragi suggested no change.');
      return undefined;
    }
    const list = await backend.hunks(now, file);
    const base = doc.uri.path.split('/').pop() || 'untitled';
    const proposed = vscode.Uri.from({ scheme: PROPOSED_SCHEME, path: `/${this.next++}/${base}`, query: doc.uri.toString() });
    // One review per document: a new one replaces the old.
    const old = this.reviews.get(doc.uri.toString());
    if (old) {
      await this.drop(old);
    }
    const r: Review = {
      original: doc.uri,
      proposed,
      code: result.code,
      file,
      hunks: list.hunks,
      generation: 1,
      backend,
      shadow: now,
      shadowVersion: doc.version,
      ours: [],
      resolved: false,
      queue: Promise.resolve(),
    };
    this.reviews.set(doc.uri.toString(), r);
    if (doc.version !== r.shadowVersion) {
      // Edited while the hunks were worked out: start from the document as it is.
      await this.enqueue(r, () => this.resync(r, doc));
    }
    await this.refresh(r);
    await vscode.commands.executeCommand('vscode.diff', doc.uri, proposed, `${base}: Bragi's rewrite (${this.summary(r)})`, {
      preview: true,
      preserveFocus: true,
      viewColumn: vscode.ViewColumn.Beside,
    });
    // Back to the document itself, where the hunks' Accept and Reject are.
    await vscode.window.showTextDocument(doc, { viewColumn: editor.viewColumn, preserveFocus: false });
    return r;
  }

  private summary(r: Review): string {
    const { added, removed } = countLines(r.hunks);
    return `${r.hunks.length} ${r.hunks.length === 1 ? 'hunk' : 'hunks'}, +${added} −${removed}`;
  }

  /** The review the user means: the one for `uri` (or the active editor), else the only one. */
  private chosen(uri?: vscode.Uri): Review | undefined {
    const active = uri ?? vscode.window.activeTextEditor?.document.uri;
    const r = active ? this.find(active) : undefined;
    if (r && !r.resolved) {
      return r;
    }
    const open = [...this.reviews.values()].filter((x) => !x.resolved);
    return open.length === 1 ? open[0] : undefined;
  }

  /** The hunk at or after the cursor, else the last one before it. */
  private hunkAtCursor(r: Review): number {
    const e = vscode.window.activeTextEditor;
    if (!e || e.document.uri.toString() !== r.original.toString() || !r.hunks.length) {
      return 0;
    }
    const line = e.selection.active.line;
    const i = r.hunks.findIndex((h) => line <= h.old_start + Math.max(h.old_len, 1) - 1);
    return i < 0 ? r.hunks.length - 1 : i;
  }

  /** Accepts one hunk: the document gets it in one edit (undo takes it back). */
  async acceptHunk(uri?: vscode.Uri, index?: number, generation?: number): Promise<boolean> {
    return this.onHunk(uri, index, generation, async (r, i) => {
      const accepted = r.hunks.map((_, k) => k === i);
      const { file } = await r.backend.applyHunks(r.shadow, r.file, accepted);
      return this.edit(r, file);
    });
  }

  /** Rejects one hunk: the proposal drops it, the document stays as it is. */
  async rejectHunk(uri?: vscode.Uri, index?: number, generation?: number): Promise<boolean> {
    return this.onHunk(uri, index, generation, async (r, i) => {
      const accepted = r.hunks.map((_, k) => k !== i);
      r.file = (await r.backend.applyHunks(r.shadow, r.file, accepted)).file;
      this.changed.fire(r.proposed);
      await this.recompute(r);
      return true;
    });
  }

  private async onHunk(uri: vscode.Uri | undefined, index: number | undefined, generation: number | undefined, act: (r: Review, i: number) => Promise<boolean>): Promise<boolean> {
    const r = this.chosen(uri);
    if (!r) {
      void vscode.window.showWarningMessage('No Bragi rewrite is waiting.');
      return false;
    }
    const ok = await this.enqueue(r, async () => {
      if (generation !== undefined && generation !== r.generation) {
        void vscode.window.showWarningMessage('That hunk changed; use the updated Accept or Reject.');
        return false;
      }
      const i = index ?? this.hunkAtCursor(r);
      if (!r.hunks[i]) {
        return false;
      }
      return act(r, i);
    });
    await this.settled(r.original);
    return ok;
  }

  /** Accept All: the rest of the proposal in one edit. */
  async accept(uri?: vscode.Uri): Promise<boolean> {
    const r = this.chosen(uri);
    if (!r) {
      void vscode.window.showWarningMessage('No Bragi rewrite is waiting.');
      return false;
    }
    await this.settled(r.original);
    const ok = await this.enqueue(r, () => this.edit(r, r.file));
    await this.settled(r.original);
    return ok;
  }

  /** Reject All: the proposal goes, the document stays as it is. */
  async reject(uri?: vscode.Uri): Promise<boolean> {
    const r = this.chosen(uri);
    if (!r) {
      return false;
    }
    await this.settled(r.original);
    await this.drop(r);
    return true;
  }

  async showDiff(uri?: vscode.Uri): Promise<void> {
    const r = this.chosen(uri);
    if (r) {
      const base = r.original.path.split('/').pop() || 'untitled';
      await vscode.commands.executeCommand('vscode.diff', r.original, r.proposed, `${base}: Bragi's rewrite (${this.summary(r)})`, { viewColumn: vscode.ViewColumn.Beside });
    }
  }

  /** Makes the document `to` with one minimal edit (one undo step). */
  private async edit(r: Review, to: string): Promise<boolean> {
    const doc = await vscode.workspace.openTextDocument(r.original);
    if (doc.version !== r.shadowVersion) {
      void vscode.window.showWarningMessage('The code is still changing; try again.');
      return false;
    }
    const m = minimalEdit(r.shadow, to);
    if (m.start === m.end && !m.text) {
      return true;
    }
    const removed = r.shadow.slice(m.start, m.end);
    const edit = new vscode.WorkspaceEdit();
    edit.replace(r.original, new vscode.Range(doc.positionAt(m.start), doc.positionAt(m.end)), m.text);
    r.ours.push({ removed, inserted: m.text });
    r.expectVersion = doc.version + 1;
    const ok = await vscode.workspace.applyEdit(edit);
    if (!ok) {
      r.ours.pop();
      r.expectVersion = undefined;
    }
    return ok;
  }

  private onEdit(e: vscode.TextDocumentChangeEvent): void {
    const r = this.reviews.get(e.document.uri.toString());
    if (!r || !e.contentChanges.length) {
      return;
    }
    const changes = [...e.contentChanges].sort((a, b) => b.rangeOffset - a.rangeOffset);
    const version = e.document.version;
    const ours = version === r.expectVersion || ((e.reason === vscode.TextDocumentChangeReason.Undo || e.reason === vscode.TextDocumentChangeReason.Redo) && this.isOurs(r, changes));
    if (version === r.expectVersion) {
      r.expectVersion = undefined;
    }
    void this.enqueue(r, async () => {
      if (!ours && r.resolved) {
        // Done, and the user went on editing: nothing left to bring back.
        await this.drop(r, false);
        return;
      }
      if (!ours) {
        // The user's own edit: the proposal gets it too where the two agree.
        let file = r.file;
        for (const c of changes) {
          const from = mapOffset(r.shadow, r.file, r.hunks, c.rangeOffset);
          const to = mapOffset(r.shadow, r.file, r.hunks, c.rangeOffset + c.rangeLength);
          if (from !== undefined && to !== undefined && from <= to) {
            file = file.slice(0, from) + c.text + file.slice(to);
          }
        }
        if (file !== r.file) {
          r.file = file;
          this.changed.fire(r.proposed);
        }
      }
      let shadow = r.shadow;
      for (const c of changes) {
        shadow = shadow.slice(0, c.rangeOffset) + c.text + shadow.slice(c.rangeOffset + c.rangeLength);
      }
      r.shadow = shadow;
      r.shadowVersion = version;
      if (e.document.version === version && e.document.getText() !== shadow) {
        await this.resync(r, e.document);
        return;
      }
      await this.recompute(r);
    });
  }

  /** Takes the document as it is (when the review lost track of an edit). */
  private async resync(r: Review, doc: vscode.TextDocument): Promise<void> {
    r.shadow = doc.getText();
    r.shadowVersion = doc.version;
    await this.recompute(r);
  }

  private isOurs(r: Review, changes: readonly vscode.TextDocumentContentChangeEvent[]): boolean {
    if (changes.length !== 1) {
      return false;
    }
    const c = changes[0];
    return r.ours.some((o) => (c.text === o.removed && c.rangeLength === o.inserted.length) || (c.text === o.inserted && c.rangeLength === o.removed.length));
  }

  private async recompute(r: Review): Promise<void> {
    try {
      r.hunks = (await r.backend.hunks(r.shadow, r.file)).hunks;
    } catch (e) {
      void vscode.window.showErrorMessage(`Bragi could not compare the rewrite: ${e instanceof Error ? e.message : String(e)}`);
      return;
    }
    r.generation++;
    const done = r.hunks.length === 0;
    if (done && !r.resolved) {
      r.resolved = true;
      await this.closeDiff(r);
    } else if (!done && r.resolved) {
      // Undo brought hunks back.
      r.resolved = false;
    }
    await this.refresh(r);
  }

  private async refresh(_r?: Review): Promise<void> {
    await this.setContext();
    this.lensesChanged.fire();
    this.decorate();
  }

  provideCodeLenses(doc: vscode.TextDocument): vscode.CodeLens[] {
    const proposedSide = doc.uri.scheme === PROPOSED_SCHEME;
    const r = proposedSide ? this.byProposed(doc.uri) : this.reviews.get(doc.uri.toString());
    if (!r || r.resolved || !r.hunks.length) {
      return [];
    }
    const at = (line: number) => new vscode.Range(Math.min(line, doc.lineCount - 1), 0, Math.min(line, doc.lineCount - 1), 0);
    const uri = r.original;
    const lenses: vscode.CodeLens[] = [
      new vscode.CodeLens(at(0), { title: `$(sparkle) Bragi: ${this.summary(r)}`, command: 'bragi.showRewriteDiff', arguments: [uri], tooltip: 'Show the whole rewrite in a diff' }),
      new vscode.CodeLens(at(0), { title: '$(check-all) Accept All', command: 'bragi.acceptRewrite', arguments: [uri] }),
      new vscode.CodeLens(at(0), { title: '$(close-all) Reject All', command: 'bragi.rejectRewrite', arguments: [uri] }),
    ];
    r.hunks.forEach((h, i) => {
      const line = proposedSide ? h.new_start + h.lines.findIndex((l) => l.tag !== 'context') : anchorLine(h);
      const added = h.lines.filter((l) => l.tag === 'added').length;
      const removed = h.lines.filter((l) => l.tag === 'removed').length;
      lenses.push(
        new vscode.CodeLens(at(line), { title: `$(check) Accept (+${added} −${removed})`, command: 'bragi.acceptHunk', arguments: [uri, i, r.generation], tooltip: hunkText(h) }),
        new vscode.CodeLens(at(line), { title: '$(close) Reject', command: 'bragi.rejectHunk', arguments: [uri, i, r.generation] }),
      );
    });
    return lenses;
  }

  /** Marks each hunk in the editors showing its document: removed lines, and what comes in. */
  private decorate(): void {
    for (const e of vscode.window.visibleTextEditors) {
      const r = this.reviews.get(e.document.uri.toString());
      if (!r || r.resolved || e.document.version !== r.shadowVersion) {
        e.setDecorations(removedLine, []);
        e.setDecorations(addedHint, []);
        continue;
      }
      const removed: vscode.DecorationOptions[] = [];
      const hints: vscode.DecorationOptions[] = [];
      for (const h of r.hunks) {
        const hover = new vscode.MarkdownString().appendCodeblock(hunkText(h), 'diff');
        const gone = h.lines.filter((l) => l.tag === 'removed');
        const added = h.lines.filter((l) => l.tag === 'added');
        for (const l of gone) {
          removed.push({ range: new vscode.Range(l.old_no! - 1, 0, l.old_no! - 1, 0), hoverMessage: hover });
        }
        if (added.length) {
          // After the last removed line, or on the line before an insertion.
          const line = gone.length ? gone[gone.length - 1].old_no! - 1 : Math.max(anchorLine(h) - 1, 0);
          if (line < e.document.lineCount) {
            const end = e.document.lineAt(line).range.end;
            const first = added[0].text.trim();
            const text = `→ ${added.length === 1 ? first.slice(0, 60) : `${added.length} lines: ${first.slice(0, 40)}…`}`;
            hints.push({ range: new vscode.Range(end, end), hoverMessage: hover, renderOptions: { after: { contentText: text } } });
          }
        }
      }
      e.setDecorations(removedLine, removed);
      e.setDecorations(addedHint, hints);
    }
  }

  private async closeDiff(r: Review): Promise<void> {
    const tabs = vscode.window.tabGroups.all
      .flatMap((g) => g.tabs)
      .filter((t) => t.input instanceof vscode.TabInputTextDiff && t.input.modified.toString() === r.proposed.toString());
    if (tabs.length) {
      await vscode.window.tabGroups.close(tabs);
    }
  }

  private async drop(r: Review, close = true): Promise<void> {
    if (this.reviews.get(r.original.toString()) === r) {
      this.reviews.delete(r.original.toString());
    }
    if (close) {
      await this.closeDiff(r);
    }
    await this.refresh();
  }
}
