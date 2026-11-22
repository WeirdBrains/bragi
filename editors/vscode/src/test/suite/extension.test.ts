import * as assert from 'assert';
import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import type { BragiApi } from '../../extension';
import { editorContext } from '../../context';
import { BragiError } from '../../types';
import { MockServer, TOKEN } from './mockServer';
import { applyPick } from '../../extension';
import { shot } from './shots';

const cfg = () => vscode.workspace.getConfiguration('bragi');
const set = (key: string, value: unknown) => cfg().update(key, value, vscode.ConfigurationTarget.Global);

async function editor(content: string, language = 'rust'): Promise<vscode.TextEditor> {
  const doc = await vscode.workspace.openTextDocument({ content, language });
  return vscode.window.showTextDocument(doc);
}

const token = () => new vscode.CancellationTokenSource().token;
const inlineCtx = { triggerKind: vscode.InlineCompletionTriggerKind.Invoke, selectedCompletionInfo: undefined } as vscode.InlineCompletionContext;
const suggests = (m: MockServer) => m.seen.filter((s) => s.path === '/v1/code/suggest').length;

async function until(what: string, f: () => boolean): Promise<void> {
  const deadline = Date.now() + 10_000;
  while (!f()) {
    assert.ok(Date.now() < deadline, `waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

suite('Bragi extension', () => {
  const mock = new MockServer();
  let api: BragiApi;

  suiteSetup(async () => {
    await mock.start();
    const ext = vscode.extensions.getExtension<BragiApi>('weirdbrains.bragi');
    assert.ok(ext, 'the extension is installed');
    api = await ext.activate();
    await set('serveUrl', mock.url);
    await set('inline.debounceMs', 0);
    await set('target', 'bragi');
    await vscode.commands.executeCommand('bragi.setToken', TOKEN);
  });

  suiteTeardown(async () => {
    await mock.stop();
  });

  teardown(async () => {
    await vscode.commands.executeCommand('workbench.action.closeAllEditors');
  });

  test('its commands are there', async () => {
    const all = await vscode.commands.getCommands(true);
    for (const c of ['bragi.rewriteSelection', 'bragi.rewriteFile', 'bragi.acceptRewrite', 'bragi.rejectRewrite', 'bragi.openChat', 'bragi.pickModel', 'bragi.setToken', 'bragi.importToken']) {
      assert.ok(all.includes(c), c);
    }
  });

  test('the context holds the selection, the text around it and the problems', async () => {
    const e = await editor('fn add(a: i32, b: i32) -> i32 {\n    a - b\n}\n');
    e.selection = new vscode.Selection(1, 4, 1, 9);
    const diags = vscode.languages.createDiagnosticCollection('bragi-test');
    diags.set(e.document.uri, [new vscode.Diagnostic(new vscode.Range(1, 4, 1, 9), 'suspicious subtraction', vscode.DiagnosticSeverity.Warning)]);
    try {
      const ctx = editorContext(e);
      assert.strictEqual(ctx.selection, 'a - b');
      assert.strictEqual(ctx.before + ctx.selection + ctx.after, e.document.getText());
      assert.strictEqual(ctx.language, 'rust');
      assert.deepStrictEqual(ctx.diagnostics.map((d) => [d.line, d.severity, d.message]), [[2, 'warning', 'suspicious subtraction']]);
    } finally {
      diags.dispose();
    }
  });

  test('inline suggestions come from bragi serve with the token', async () => {
    const e = await editor('fn add(a: i32, b: i32) -> i32 {\n    let total = \n}\n');
    const pos = new vscode.Position(1, 16);
    const items = await api.inline.provideInlineCompletionItems(e.document, pos, inlineCtx, token());
    assert.strictEqual(items.length, 1);
    assert.strictEqual(items[0].insertText, 'a + b');
    const seen = mock.last('/v1/code/suggest')!;
    assert.strictEqual(seen.auth, `Bearer ${TOKEN}`);
    assert.strictEqual(seen.body.context.before, 'fn add(a: i32, b: i32) -> i32 {\n    let total = ');
    assert.strictEqual(seen.body.context.selection, '');
    assert.strictEqual(seen.body.context.after, '\n}\n');
  });

  test('a cancelled inline request asks nothing', async () => {
    const e = await editor('x');
    const src = new vscode.CancellationTokenSource();
    src.cancel();
    const before = suggests(mock);
    const items = await api.inline.provideInlineCompletionItems(e.document, new vscode.Position(0, 1), inlineCtx, src.token);
    assert.deepStrictEqual(items, []);
    assert.strictEqual(suggests(mock), before);
  });

  test('a selection rewrite shows a diff and accepting applies it', async () => {
    const e = await editor('fn add(a: i32, b: i32) -> i32 {\n    a - b\n}\n');
    e.selection = new vscode.Selection(1, 0, 2, 0);
    await vscode.commands.executeCommand('bragi.rewriteSelection', 'shout');
    assert.strictEqual(api.rewrites.list().length, 1);
    const p = api.rewrites.list()[0];
    assert.strictEqual(p.code, '    A - B\n');
    assert.strictEqual(p.file, 'fn add(a: i32, b: i32) -> i32 {\n    A - B\n}\n');
    const proposed = await vscode.workspace.openTextDocument(p.proposed);
    assert.strictEqual(proposed.getText(), p.file, 'the diff shows the proposed file');
    await until('the diff tab', () => vscode.window.tabGroups.all.some((g) => g.tabs.some((t) => t.input instanceof vscode.TabInputTextDiff)));
    assert.strictEqual(await vscode.commands.executeCommand('bragi.acceptRewrite'), true);
    assert.strictEqual(e.document.getText(), 'fn add(a: i32, b: i32) -> i32 {\n    A - B\n}\n');
    assert.strictEqual(api.rewrites.list().length, 0);
  });

  test('rejecting a rewrite leaves the code alone', async () => {
    const e = await editor('let a = 1;\n', 'javascript');
    await vscode.commands.executeCommand('bragi.rewriteFile', 'shout');
    assert.strictEqual(api.rewrites.list()[0].file, 'LET A = 1;\n');
    assert.strictEqual(await vscode.commands.executeCommand('bragi.rejectRewrite'), true);
    assert.strictEqual(e.document.getText(), 'let a = 1;\n');
    assert.strictEqual(api.rewrites.list().length, 0);
  });

  test('an accepted rewrite still lands after an unrelated edit', async () => {
    const e = await editor('a\nb\nc\n', 'plaintext');
    e.selection = new vscode.Selection(1, 0, 1, 1);
    await vscode.commands.executeCommand('bragi.rewriteSelection', 'shout');
    // The diff now has focus, so edit the document the way another tool would.
    const edit = new vscode.WorkspaceEdit();
    edit.insert(e.document.uri, new vscode.Position(0, 0), 'top\n');
    assert.ok(await vscode.workspace.applyEdit(edit));
    assert.strictEqual(await vscode.commands.executeCommand('bragi.acceptRewrite'), true);
    assert.strictEqual(e.document.getText(), 'top\na\nB\nc\n');
  });

  test('chat streams the answer and continues its conversation', async () => {
    const e = await editor('fn add(a: i32, b: i32) -> i32 { a + b }\n');
    const text = await api.chat.controller.send('What does add do?', editorContext(e));
    assert.strictEqual(text, 'It adds two numbers.');
    assert.strictEqual(api.chat.controller.conversationId, 'c-test');
    const first = mock.last('/v1/code/chat')!;
    assert.strictEqual(first.body.prompt, 'What does add do?');
    const ctx = first.body.context;
    assert.strictEqual(ctx.before + ctx.selection + ctx.after, e.document.getText());
    assert.strictEqual(first.body.conversation, undefined);
    await api.chat.controller.send('And sub?');
    assert.strictEqual(mock.last('/v1/code/chat')!.body.conversation, 'c-test');
    api.chat.controller.reset();
    assert.strictEqual(api.chat.controller.conversationId, undefined);
  });

  test('the model setting goes with requests', async () => {
    await set('model', 'qwen3-32b');
    try {
      const e = await editor('x = ');
      await api.inline.provideInlineCompletionItems(e.document, new vscode.Position(0, 4), inlineCtx, token());
      assert.strictEqual(mock.last('/v1/code/suggest')!.body.model, 'qwen3-32b');
      const list = await api.router.bragi.models();
      assert.deepStrictEqual(list.models.map((m) => m.id), ['gemma-4-e4b', 'qwen3-32b']);
      assert.strictEqual(list.defaultModel, 'gemma-4-e4b');
    } finally {
      await set('model', '');
    }
  });

  test('a target from Bragi Settings goes with code and chat requests', async () => {
    const targets = await api.router.bragi.targets();
    assert.deepStrictEqual(targets.map((t) => t.target.kind), ['pool', 'endpoint']);
    await applyPick({ target: 'bragi', model: '', bragiTarget: targets[1].target });
    try {
      const e = await editor('x = ');
      await api.inline.provideInlineCompletionItems(e.document, new vscode.Position(0, 4), inlineCtx, token());
      assert.deepStrictEqual(mock.last('/v1/code/suggest')!.body.target, { kind: 'endpoint', id: 'e-1', model: 'qwen3.6-35b-a3b' });
      await api.chat.controller.send('hi');
      assert.deepStrictEqual(mock.last('/v1/code/chat')!.body.target, { kind: 'endpoint', id: 'e-1', model: 'qwen3.6-35b-a3b' });
    } finally {
      api.chat.controller.reset();
      await applyPick({ target: 'bragi', model: '', bragiTarget: null });
    }
    assert.strictEqual(cfg().get('bragiTarget'), null);
  });

  test('screenshots of the suggestion, the diff and the chat (Xvfb only)', async function () {
    if (!process.env.BRAGI_SHOTS) {
      this.skip();
    }
    // A ghost-text suggestion in the editor.
    await set('inline.enabled', true);
    const e = await editor('fn add(a: i32, b: i32) -> i32 {\n    let total = \n    total\n}\n');
    e.selection = new vscode.Selection(1, 16, 1, 16);
    await vscode.commands.executeCommand('editor.action.inlineSuggest.trigger');
    await new Promise((r) => setTimeout(r, 2500));
    shot('01-inline-suggestion');
    // A rewrite waiting in the diff.
    e.selection = new vscode.Selection(2, 0, 3, 0);
    await vscode.commands.executeCommand('bragi.rewriteSelection', 'shout');
    await new Promise((r) => setTimeout(r, 2000));
    shot('02-rewrite-diff');
    await vscode.commands.executeCommand('bragi.rejectRewrite');
    // The chat panel with an answer.
    await vscode.commands.executeCommand('bragi.openChat');
    await new Promise((r) => setTimeout(r, 2000));
    await api.chat.controller.send('What does add do?', editorContext(e));
    await new Promise((r) => setTimeout(r, 2000));
    shot('03-chat-panel');
    api.chat.controller.reset();
  });

  test('without a token it says how to get one, and a token file is imported', async () => {
    const tmp = process.env.BRAGI_TEST_TMP!;
    await api.secrets.clearServeToken();
    await set('tokenFile', path.join(tmp, 'missing-token'));
    try {
      await assert.rejects(
        api.router.bragi.suggest({ path: 'a', language: '', before: '', selection: '', after: '', open_files: [], diagnostics: [] }, undefined),
        (e: BragiError) => e.kind === 'unauthorized' && /Import Token/.test(e.message),
      );
      const file = path.join(tmp, 'token');
      fs.writeFileSync(file, TOKEN + '\n');
      assert.strictEqual(await vscode.commands.executeCommand('bragi.importToken', file), true);
      assert.strictEqual(await api.secrets.serveToken(), TOKEN);
    } finally {
      await set('tokenFile', '');
      await api.secrets.setServeToken(TOKEN);
    }
  });

  test('a wrong token is reported as unauthorized', async () => {
    await api.secrets.setServeToken('0'.repeat(64));
    try {
      await assert.rejects(api.router.bragi.models(), (e: BragiError) => e.kind === 'unauthorized' && e.status === 401);
    } finally {
      await api.secrets.setServeToken(TOKEN);
    }
  });

  test('bragi.serveUrl must be loopback', async () => {
    await set('serveUrl', 'http://example.com:4877');
    try {
      await assert.rejects(api.router.bragi.models(), /loopback/);
    } finally {
      await set('serveUrl', mock.url);
    }
  });

  test('the direct target talks OpenAI to the configured server', async () => {
    await set('target', 'direct');
    await set('direct.url', `${mock.url}/openai/v1`);
    try {
      const e = await editor('let total = ');
      const items = await api.inline.provideInlineCompletionItems(e.document, new vscode.Position(0, 12), inlineCtx, token());
      assert.strictEqual(items[0]?.insertText, 'a + b');
      const seen = mock.last('/openai/v1/chat/completions')!;
      assert.strictEqual(seen.body.model, 'direct-model', "the server's first model");
      assert.strictEqual(seen.body.messages[0].role, 'system');
      assert.ok(seen.body.messages[1].content.includes('<code_before_cursor>\nlet total = </code_before_cursor>'));
      const text = await api.chat.controller.send('hi');
      assert.strictEqual(text, 'Hello from direct');
    } finally {
      api.chat.controller.reset();
      await set('target', 'bragi');
      await set('direct.url', '');
    }
  });

  test('with the fallback on, an unreachable bragi serve falls back to the direct server', async () => {
    await set('serveUrl', 'http://127.0.0.1:9');
    await set('direct.url', `${mock.url}/openai/v1`);
    await set('fallbackToDirect', true);
    try {
      const b = await api.router.backend();
      assert.strictEqual(b.id, 'direct');
    } finally {
      await set('fallbackToDirect', false);
      await set('direct.url', '');
      await set('serveUrl', mock.url);
    }
  });
});
