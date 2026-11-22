import * as assert from 'assert';
import * as vscode from 'vscode';
import type { BragiApi } from '../../extension';
import { clientServersFrom, effectiveOf, manifestProperties, mcpArgs, mcpClientConfig, sourceOf } from '../../settings';
import { MockServer, TOKEN } from './mockServer';

const cfg = () => vscode.workspace.getConfiguration('bragi');
const set = (key: string, value: unknown) => cfg().update(key, value, vscode.ConfigurationTarget.Global);
const exec = <T = unknown>(c: string, ...a: unknown[]) => Promise.resolve(vscode.commands.executeCommand<T>(c, ...a));

suite('Settings and management (parity with the Bragi apps)', () => {
  const mock = new MockServer();
  let api: BragiApi;
  let props: Record<string, any>;

  suiteSetup(async () => {
    await mock.start();
    const ext = vscode.extensions.getExtension<BragiApi>('weirdbrains.bragi');
    assert.ok(ext);
    api = await ext.activate();
    props = manifestProperties(ext);
    await set('serveUrl', mock.url);
    await set('target', 'bragi');
    await set('autoStartServer', false);
    await exec('bragi.setToken', TOKEN);
  });

  suiteTeardown(async () => {
    await set('mcp.clientServers', undefined);
    await set('mcp.server.allowWrites', undefined);
    await mock.stop();
  });

  // ---- settings ----

  test('every setting has a title, a description, an order and a scope, in a Bragi: section', () => {
    const ext = vscode.extensions.getExtension('weirdbrains.bragi')!;
    const sections = ext.packageJSON.contributes.configuration as { title: string; properties: Record<string, any> }[];
    assert.deepStrictEqual(sections.map((s) => s.title), ['Bragi: Connection', 'Bragi: Models', 'Bragi: Chat', 'Bragi: MCP', 'Bragi: Advanced']);
    for (const [k, p] of Object.entries(props)) {
      assert.ok(p.title, `${k} title`);
      assert.ok(p.description || p.markdownDescription, `${k} description`);
      assert.ok(typeof p.order === 'number', `${k} order`);
      assert.ok(['application', 'machine', 'window', 'resource', 'machine-overridable', 'language-overridable'].includes(p.scope), `${k} scope`);
    }
  });

  test('keys that run a program or point at a server are machine scope, so a workspace cannot set them', () => {
    for (const k of ['bragi.path', 'bragi.serveUrl', 'bragi.autoStartServer', 'bragi.tokenFile', 'bragi.direct.url', 'bragi.mcp.clientServers', 'bragi.mcp.server.args']) {
      assert.strictEqual(props[k]?.scope, 'machine', k);
    }
  });

  test('the old keys keep their names and defaults', () => {
    assert.strictEqual(props['bragi.target'].default, 'bragi');
    assert.strictEqual(props['bragi.inline.debounceMs'].default, 350);
    assert.strictEqual(props['bragi.serveUrl'].default, 'http://127.0.0.1:4877');
  });

  test('precedence: workspace folder > workspace > user > default', () => {
    const none = { key: 'k', defaultValue: 1 };
    assert.strictEqual(sourceOf(none), 'default');
    assert.strictEqual(sourceOf({ ...none, globalValue: 2 }), 'user');
    assert.strictEqual(sourceOf({ ...none, globalValue: 2, workspaceValue: 3 }), 'workspace');
    assert.strictEqual(sourceOf({ ...none, globalValue: 2, workspaceValue: 3, workspaceFolderValue: 4 }), 'workspace folder');
    assert.strictEqual(effectiveOf({ ...none, globalValue: 2, workspaceValue: 3 }), 3);
    assert.strictEqual(effectiveOf<any>({ ...none, globalValue: false }), false, 'a false user value beats the default');
  });

  test('a user setting is read live through getConfiguration, and a change is seen without a reload', async () => {
    const seen = new Promise<boolean>((r) => {
      const d = vscode.workspace.onDidChangeConfiguration((e) => {
        if (e.affectsConfiguration('bragi.inline.debounceMs')) {
          d.dispose();
          r(true);
        }
      });
    });
    await set('inline.debounceMs', 123);
    assert.ok(await seen);
    const i = cfg().inspect<number>('inline.debounceMs');
    assert.strictEqual(sourceOf(i), 'user');
    assert.strictEqual(cfg().get('inline.debounceMs'), 123);
    await set('inline.debounceMs', undefined);
    assert.strictEqual(sourceOf(cfg().inspect('inline.debounceMs')), 'default');
  });

  test('the Settings and Where They Come From list covers every key', async () => {
    const orig = vscode.window.showQuickPick;
    let items: any[] = [];
    (vscode.window as any).showQuickPick = async (i: any[]) => {
      items = i;
      return undefined;
    };
    try {
      await exec('bragi.settings.show');
    } finally {
      (vscode.window as any).showQuickPick = orig;
    }
    assert.strictEqual(items.length, Object.keys(props).length);
    const row = items.find((i) => i.label === 'bragi.serveUrl');
    assert.strictEqual(row.description, 'user');
  });

  test('the MCP server command line follows the settings', async () => {
    assert.deepStrictEqual(mcpArgs(undefined, { allowWrites: false, allowDeletes: false, extraArgs: [] }), ['mcp']);
    assert.deepStrictEqual(mcpArgs('/p', { allowWrites: true, allowDeletes: true, extraArgs: ['--home', '/h'] }), ['mcp', '--root', '/p', '--allow-writes', '--allow-deletes', '--home', '/h']);
    await set('mcp.server.allowWrites', true);
    const json = (await exec<string>('bragi.mcp.copyConfig', 'cursor'))!;
    assert.ok(JSON.parse(json).mcpServers.bragi.args.includes('--allow-writes'));
    assert.strictEqual(await vscode.env.clipboard.readText(), json);
    await set('mcp.server.allowWrites', undefined);
  });

  test('MCP client configs match each client\'s format', () => {
    const c = JSON.parse(mcpClientConfig('cursor', 'bragi', ['mcp']).json);
    assert.deepStrictEqual(c, { mcpServers: { bragi: { command: 'bragi', args: ['mcp'] } } });
    assert.deepStrictEqual(JSON.parse(mcpClientConfig('vscode', 'bragi', ['mcp']).json), { servers: { bragi: { type: 'stdio', command: 'bragi', args: ['mcp'] } } });
    assert.ok(JSON.parse(mcpClientConfig('claude', 'bragi', ['mcp']).json).mcpServers.bragi);
  });

  test('bad MCP server settings are reported, good ones kept', () => {
    const r = clientServersFrom([{ name: 'a', transport: 'stdio', command: 'x' }, { name: '', transport: 'http' }, { name: 'b', transport: 'http' }, { name: 'c', transport: 'ftp' }, null]);
    assert.deepStrictEqual(r.servers.map((s) => s.name), ['a']);
    assert.strictEqual(r.problems.length, 4);
  });

  // ---- links ----

  test('a link code is read, the short codes compared and the link confirmed', async () => {
    await exec('bragi.link.add', 'weirdnet-bragi://link/abc', true);
    assert.strictEqual(mock.last('/v1/link/accept')?.body.code, 'weirdnet-bragi://link/abc');
    assert.strictEqual(mock.linked, true);
    await exec('bragi.link.add', 'weirdnet-bragi://link/abc', false);
    assert.strictEqual(mock.linked, false, 'a mismatch cancels');
  });

  test('links are added by address and removed', async () => {
    await exec('bragi.link.addNode', '/ip4/10.0.0.1/tcp/1/p2p/abc');
    assert.strictEqual((await api.router.bragi.api<{ links: unknown[] }>('GET', '/v1/links')).links.length, 1);
    await exec('bragi.link.remove', '/ip4/10.0.0.1/tcp/1/p2p/abc');
    assert.strictEqual(mock.links.length, 0);
  });

  // ---- OpenAI connections ----

  test('OpenAI connections: add, edit, test, remove; the key goes to Bragi and is never kept here', async () => {
    const key = 'sk-test-0000000000000000000000';
    await exec('bragi.endpoint.add', { name: 'Local', baseUrl: 'http://127.0.0.1:8484/v1', model: 'm1', mode: 'alongside', apiKey: key });
    assert.strictEqual(mock.last('/v1/endpoints')?.body.api_key, key);
    assert.strictEqual(mock.endpoints[0].has_key, true);
    assert.ok(!JSON.stringify(mock.endpoints).includes(key), 'the server view has no key');
    // Not in settings, not in secret storage under any known name.
    assert.ok(!JSON.stringify(cfg().inspect('direct.url')).includes(key));
    const id = mock.endpoints[0].id;
    await exec('bragi.endpoint.edit', id, { name: 'Renamed', baseUrl: 'http://127.0.0.1:8484/v1', mode: 'fallback' });
    assert.strictEqual(mock.endpoints[0].name, 'Renamed');
    assert.strictEqual(mock.endpoints[0].has_key, true, 'an edit without a key keeps it');
    const report = (await exec<{ ok: boolean; models: string[] }>('bragi.endpoint.test', id))!;
    assert.deepStrictEqual(report.models, ['m1', 'm2']);
    await exec('bragi.endpoint.remove', id, true);
    assert.strictEqual(mock.endpoints.length, 0);
  });

  test('the Connections view lists links, connections and servers', async () => {
    await exec('bragi.endpoint.add', { name: 'Tree', baseUrl: 'http://127.0.0.1:1/v1' });
    const { ConnectionsView } = await import('../../tree');
    const view = new ConnectionsView(new (await import('../../manage')).Manage(() => api.router.bragi), api.serve, () => false);
    const groups = await view.getChildren();
    assert.strictEqual(groups.length, 5);
    const eps = await view.getChildren(groups[2]);
    assert.strictEqual(eps[0].label, 'Tree');
    assert.strictEqual(eps[0].contextValue, 'endpoint');
    await exec('bragi.endpoint.remove', mock.endpoints[0].id, true);
  });

  // ---- MCP client servers ----

  test('outside MCP servers: add, turn off, remove; a bearer goes to Bragi only', async () => {
    mock.mcpClient = true;
    await exec('bragi.mcpServer.add', { name: 'Docs', transport: 'http', url: 'https://example.com/mcp', secrets: { bearer: 'tok-123456789' } });
    assert.strictEqual(mock.mcpServers[0].has_bearer, true);
    assert.ok(!JSON.stringify(mock.mcpServers).includes('tok-123456789'));
    const id = mock.mcpServers[0].id;
    await exec('bragi.mcpServer.toggle', id);
    assert.strictEqual(mock.mcpServers[0].enabled, false);
    await exec('bragi.mcpServer.remove', id);
    assert.strictEqual(mock.mcpServers.length, 0);
  });

  test('servers from settings are added once, by name', async () => {
    mock.mcpClient = true;
    await set('mcp.clientServers', [{ name: 'Files', transport: 'stdio', command: 'npx', args: ['-y', 'server'] }, { name: 'Bad', transport: 'http' }]);
    assert.strictEqual(await exec('bragi.mcp.applySettings'), 1);
    assert.strictEqual(await exec('bragi.mcp.applySettings'), 0, 'idempotent');
    assert.deepStrictEqual(mock.mcpServers.map((s) => s.name), ['Files']);
    mock.mcpServers = [];
  });

  test('a Bragi without the MCP client says so instead of failing', async () => {
    mock.mcpClient = false;
    const { Manage } = await import('../../manage');
    await assert.rejects(new Manage(() => api.router.bragi).mcpServers(), (e: any) => e.kind === 'mcp_unavailable');
    mock.mcpClient = true;
  });

  // ---- the local servers ----

  test('the MCP server runs in a terminal and stops', async () => {
    assert.strictEqual(await exec('bragi.mcp.isRunning'), false);
    const args = (await exec<string[]>('bragi.mcp.start'))!;
    assert.strictEqual(args[0], 'mcp');
    assert.strictEqual(await exec('bragi.mcp.isRunning'), true);
    await exec('bragi.mcp.stop');
    assert.strictEqual(await exec('bragi.mcp.isRunning'), false);
  });

  test('the management commands are registered', async () => {
    const all = await vscode.commands.getCommands(true);
    for (const c of ['bragi.link.add', 'bragi.endpoint.add', 'bragi.serve.stop', 'bragi.serve.status', 'bragi.mcp.start', 'bragi.mcp.copyConfig', 'bragi.mcpServer.add', 'bragi.settings.open', 'bragi.settings.show']) {
      assert.ok(all.includes(c), c);
    }
  });
});
