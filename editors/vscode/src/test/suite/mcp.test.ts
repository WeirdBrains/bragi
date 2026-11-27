// Outside MCP servers and agent mode in the extension (LF-40 on LF-37), against a mock `bragi
// serve`: the server list in bragi-core's shape (no secrets), a folder scope, an edit that keeps
// the token, "always allow" given from a card and taken back, and agent turns in the chat whose
// approval cards are answered (allow, always, deny).

import * as assert from 'assert';
import * as vscode from 'vscode';
import type { BragiApi } from '../../extension';
import { ChatController, CHOICE_TEXT, ViewMessage } from '../../chat';
import { Manage } from '../../manage';
import { BragiError } from '../../types';
import { MockServer, TOKEN } from './mockServer';

const cfg = () => vscode.workspace.getConfiguration('bragi');
const set = (key: string, value: unknown) => cfg().update(key, value, vscode.ConfigurationTarget.Global);
const exec = <T = unknown>(cmd: string, ...args: unknown[]) => vscode.commands.executeCommand<T>(cmd, ...args) as Thenable<T>;

suite('MCP servers and agent mode', () => {
  const mock = new MockServer();
  let api: BragiApi;
  let manage: Manage;

  suiteSetup(async () => {
    await mock.start();
    const ext = vscode.extensions.getExtension<BragiApi>('weirdbrains.bragi');
    api = await ext!.activate();
    await set('serveUrl', mock.url);
    await set('target', 'bragi');
    await set('autoStartServer', false);
    await vscode.commands.executeCommand('bragi.setToken', TOKEN);
    manage = new Manage(() => api.router.bragi);
  });

  suiteTeardown(async () => {
    await mock.stop();
  });

  setup(() => {
    mock.mcpServers = [];
    mock.approvals = [];
  });

  test('a server is listed without secrets, scoped to a folder, edited without losing its token', async () => {
    const v = await manage.addMcpServer({ name: 'Docs', transport: 'http', url: 'https://example.com/mcp', scope: 'folder', folder: '/work/app', secrets: { bearer: 'tok-abcdef123456' } });
    assert.strictEqual(v.target, 'https://example.com/mcp');
    assert.deepStrictEqual(v.scope, { kind: 'folder', value: '/work/app' });
    assert.strictEqual(v.has_bearer, true);
    assert.ok(!JSON.stringify(await manage.mcpServers()).includes('tok-abcdef123456'));
    assert.deepStrictEqual(mock.last('/v1/mcp-servers')!.body.secrets, { bearer: 'tok-abcdef123456' }, 'the token goes to Bragi once');
    const e = await manage.editMcpServer(v.id, { name: 'Docs', transport: 'http', url: 'https://example.com/v2/mcp' });
    assert.strictEqual(e.target, 'https://example.com/v2/mcp');
    assert.strictEqual(e.has_bearer, true, 'no secrets in the edit: the stored token stays');
    const r = await exec<{ ok: boolean; tools: string[] }>('bragi.mcpServer.test', v.id);
    assert.deepStrictEqual(r.tools, ['docs.search']);
  });

  test('"always allow" is taken back by command', async () => {
    const v = await manage.addMcpServer({ name: 'Docs', transport: 'http', url: 'https://example.com/mcp' });
    assert.deepStrictEqual((await manage.setMcpToolAlwaysAllowed(v.id, 'docs.search', true)).always_allow, ['docs.search']);
    await exec('bragi.mcpServer.alwaysAllow', v.id, 'docs.search');
    assert.deepStrictEqual(mock.mcpServers[0].always_allow, []);
    assert.deepStrictEqual(mock.last(`/v1/mcp-servers/${v.id}/always-allow`)!.body, { tool: 'docs.search', allow: false });
  });

  /** Sends an agent turn and answers its card with `choice`. */
  async function agentTurn(choice: keyof typeof CHOICE_TEXT): Promise<{ text: string; seen: ViewMessage[]; c: ChatController }> {
    const seen: ViewMessage[] = [];
    let c!: ChatController;
    c = new ChatController(
      api.router,
      (m) => {
        seen.push(m);
        if (m.type === 'approval') {
          setTimeout(() => void c.approve(m.ask, choice), 10);
        }
      },
      () => true,
      () => '/work/app',
    );
    const text = await c.send('What does add do?', undefined, { agent: true });
    return { text, seen, c };
  }

  test('an agent turn shows the card; "always allow" lets the tool run and remembers it', async () => {
    await manage.addMcpServer({ name: 'Docs', transport: 'http', url: 'https://example.com/mcp' });
    const { text, seen, c } = await agentTurn('always');
    const sent = mock.last('/v1/code/chat')!.body;
    assert.strictEqual(sent.agent, true);
    assert.strictEqual(sent.folder, '/work/app', 'the workspace folder, for folder-scoped servers');
    const card = seen.find((m) => m.type === 'approval') as Extract<ViewMessage, { type: 'approval' }>;
    assert.deepStrictEqual([card.tool, card.tier, card.summary, card.always], ['docs.search', 'read', 'Search the docs for "add"', true]);
    assert.deepStrictEqual(mock.approvals, [{ request_id: 'r-agent', ask: 1, choice: 'always' }]);
    const order = seen.map((m) => (m.type === 'tool' ? `tool:${m.state}` : m.type)).filter((t) => ['approval', 'approved', 'tool:started', 'tool:done', 'done'].includes(t));
    assert.deepStrictEqual(order, ['approval', 'approved', 'tool:started', 'tool:done', 'done']);
    assert.strictEqual(text, 'Let me look. The docs say add sums.');
    assert.deepStrictEqual(mock.mcpServers[0].always_allow, ['docs.search']);
    assert.strictEqual(await c.approve(1, 'once'), false, 'the task ended: the card is gone');
  });

  test('a denied card stops the tool and says so', async () => {
    const { text, seen } = await agentTurn('deny');
    assert.strictEqual(text, 'Let me look. I could not search.');
    assert.ok(!seen.some((m) => m.type === 'tool'));
    assert.ok(seen.some((m) => m.type === 'notice' && /declined/.test(m.text)));
    assert.ok(seen.some((m) => m.type === 'approved' && m.choice === 'deny'));
  });

  test('a plain chat is not an agent turn, and the direct target refuses one', async () => {
    const c = new ChatController(api.router, () => undefined);
    await c.send('hi');
    assert.strictEqual(mock.last('/v1/code/chat')!.body.agent, undefined);
    assert.strictEqual(await c.approve(1, 'once'), false, 'no card is waiting');
    await assert.rejects(api.router.direct.chat({ prompt: 'x', agent: true }, () => undefined), (e: BragiError) => /Agent mode needs the Bragi target/.test(e.message));
  });

  test('its command is there', async () => {
    assert.ok((await vscode.commands.getCommands(true)).includes('bragi.mcpServer.alwaysAllow'));
  });
});
