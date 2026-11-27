// The rest of extension parity (LF-40): files, the selection, images and sound in the chat panel;
// reactions and deleting a message (with undo); opening a chat from history; chat folders; and a
// link code shown for another device. Against a mock `bragi serve`.

import * as assert from 'assert';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as vscode from 'vscode';
import type { BragiApi } from '../../extension';
import { ChatController, ViewMessage } from '../../chat';
import { folderPaths, sessionItems } from '../../commands';
import { inlineText, MAX_IMAGE_BYTES, readChatFile, selectionFile, wireFile } from '../../files';
import { BragiError } from '../../types';
import { MockServer, TOKEN } from './mockServer';

const cfg = () => vscode.workspace.getConfiguration('bragi');
const set = (key: string, value: unknown) => cfg().update(key, value, vscode.ConfigurationTarget.Global);
const exec = <T = unknown>(cmd: string, ...args: unknown[]) => vscode.commands.executeCommand<T>(cmd, ...args) as Thenable<T>;

const PNG = Buffer.concat([Buffer.from('89504e470d0a1a0a', 'hex'), Buffer.alloc(16)]);
const WAV = Buffer.concat([Buffer.from('RIFF\x24\x00\x00\x00WAVEfmt ', 'latin1'), Buffer.alloc(16)]);

suite('Chat files, reactions, history, folders and link codes', () => {
  const mock = new MockServer();
  let api: BragiApi;
  let dir: string;
  const file = (name: string, data: string | Buffer) => {
    const p = path.join(dir, name);
    fs.writeFileSync(p, data);
    return p;
  };

  suiteSetup(async () => {
    await mock.start();
    const ext = vscode.extensions.getExtension<BragiApi>('weirdbrains.bragi');
    api = await ext!.activate();
    await set('serveUrl', mock.url);
    await set('target', 'bragi');
    await set('autoStartServer', false);
    await vscode.commands.executeCommand('bragi.setToken', TOKEN);
    dir = fs.mkdtempSync(path.join(process.env.BRAGI_TEST_TMP || os.tmpdir(), 'chatfiles-'));
  });

  suiteTeardown(async () => {
    await mock.stop();
    fs.rmSync(dir, { recursive: true, force: true });
  });

  setup(() => {
    mock.chats.clear();
    mock.folders = [];
    mock.sessionFolders.clear();
  });

  test('files are read as text, images or sound, and refused when Bragi would refuse them', () => {
    const t = readChatFile(file('notes.md', '# Notes\nadd sums\n'));
    assert.deepStrictEqual(wireFile(t), { name: 'notes.md', text: '# Notes\nadd sums\n' });
    const img = readChatFile(file('shot.png', PNG));
    assert.strictEqual(img.kind, 'image');
    assert.strictEqual(img.src, `data:image/png;base64,${PNG.toString('base64')}`);
    assert.deepStrictEqual(wireFile(img), { name: 'shot.png', data: PNG.toString('base64') });
    const snd = readChatFile(file('hi.wav', WAV));
    assert.strictEqual(snd.kind, 'audio');
    assert.ok(snd.src!.startsWith('data:audio/wav;base64,'));
    assert.throws(() => readChatFile(file('a.bin', Buffer.from([1, 0, 2]))), /not a text file/);
    assert.throws(() => readChatFile(path.join(dir, 'missing.txt')), /can't be read/);
    assert.throws(() => readChatFile('/big.png', () => Buffer.alloc(MAX_IMAGE_BYTES + 1)), /KiB for an image/);
    assert.strictEqual(selectionFile('src/math.rs', 3, 5, 'a + b').name, 'selection in src/math.rs:3-5');
    assert.strictEqual(selectionFile('src/math.rs', 3, 3, 'a').name, 'selection in src/math.rs:3');
    assert.throws(() => selectionFile('x', 1, 1, '  '), /Select some text/);
    assert.strictEqual(inlineText([t]), 'notes.md:\n```\n# Notes\nadd sums\n\n```\n');
  });

  test('attached files, a selection and an image go with the message and show in it', async () => {
    const posted: ViewMessage[] = [];
    const c = new ChatController(api.router, (m) => posted.push(m));
    c.attachFile(file('notes.md', 'add sums'));
    c.attachFile(file('shot.png', PNG));
    c.attachSelection('src/math.rs', 1, 2, 'pub fn add() {}\n');
    assert.deepStrictEqual(
      c.pendingFiles.map((f) => f.name),
      ['notes.md', 'shot.png', 'selection in src/math.rs:1-2'],
    );
    c.detachFile(0);
    c.attachFile(file('notes.md', 'add sums'));
    const files = posted.filter((m) => m.type === 'files').pop() as { files: { name: string }[] };
    assert.deepStrictEqual(files.files.map((f) => f.name), ['shot.png', 'selection in src/math.rs:1-2', 'notes.md']);

    await c.send('what do these say?');
    const body = mock.last('/v1/code/chat')!.body;
    assert.deepStrictEqual(body.attachments, [
      { name: 'shot.png', data: PNG.toString('base64') },
      { name: 'selection in src/math.rs:1-2', text: 'pub fn add() {}\n' },
      { name: 'notes.md', text: 'add sums' },
    ]);
    const user = posted.find((m) => m.type === 'user') as { files: { name: string; kind: string; src?: string }[] };
    assert.strictEqual(user.files[0].kind, 'image');
    assert.ok(user.files[0].src!.startsWith('data:image/png;base64,'));
    assert.strictEqual(c.pendingFiles.length, 0, 'sent files are not sent again');
    const ids = posted.find((m) => m.type === 'ids') as { user: string; answer: string };
    assert.ok(ids.user && ids.answer, 'the question and answer get their ids for reactions and delete');
    assert.ok(posted.findIndex((m) => m.type === 'done') < posted.findIndex((m) => m.type === 'ids'));
  });

  test('a reaction toggles, a delete takes the answer too and undo puts both back', async () => {
    const posted: ViewMessage[] = [];
    const c = new ChatController(api.router, (m) => posted.push(m));
    await assert.rejects(c.react('x', '👍'), (e: BragiError) => /kept in Bragi/.test(e.message));
    c.attachFile(file('hi.wav', WAV));
    await c.send('hello');
    const { user, answer } = posted.find((m) => m.type === 'ids') as { user: string; answer: string };
    assert.deepStrictEqual(await c.react(answer, '👍'), ['👍']);
    assert.deepStrictEqual(posted.pop(), { type: 'reactions', id: answer, reactions: ['👍'] });
    assert.deepStrictEqual(await c.react(answer, '👍'), []);

    const undo = await c.deleteMessage(user, true);
    assert.deepStrictEqual(posted.pop(), { type: 'removed', ids: [user, answer] });
    assert.strictEqual(mock.chats.get('c-test')!.length, 0);
    posted.length = 0;
    await c.undo(undo);
    assert.strictEqual(posted[0].type, 'reset');
    const shown = posted.filter((m) => m.type === 'message') as { id: string; role: string; files?: { kind: string; src?: string }[] }[];
    assert.deepStrictEqual(shown.map((m) => [m.id, m.role]), [[user, 'user'], [answer, 'assistant']]);
    assert.strictEqual(shown[0].files![0].kind, 'audio');
    assert.ok(shown[0].files![0].src!.startsWith('data:audio/wav;base64,'));
  });

  test('a chat opens from history with its messages, and the next message continues it', async () => {
    mock.chats.set('c-old', [
      { id: 'u1', role: 'user', text: 'What is add?', state: 'complete', attachments: [{ name: 'x.png', text: '', media: { kind: 'image', mime: 'image/png', data: 'AA==' } }] },
      { id: 'a1', role: 'assistant', text: 'It sums.', state: 'complete', model: 'gemma-4-e4b', reactions: ['❤️'] },
    ]);
    const posted: ViewMessage[] = [];
    const c = new ChatController(api.router, (m) => posted.push(m));
    assert.strictEqual(await c.open('c-old'), 2);
    assert.strictEqual(c.conversationId, 'c-old');
    assert.deepStrictEqual(posted[1], { type: 'message', role: 'user', id: 'u1', text: 'What is add?', files: [{ name: 'x.png', kind: 'image', src: 'data:image/png;base64,AA==' }], reactions: undefined, model: undefined });
    assert.deepStrictEqual(posted[2], { type: 'message', role: 'assistant', id: 'a1', text: 'It sums.', files: undefined, reactions: ['❤️'], model: 'gemma-4-e4b' });
    await c.send('and subtract?');
    assert.strictEqual(mock.last('/v1/code/chat')!.body.conversation, 'c-old');
    await assert.rejects(c.open('nope'), (e: BragiError) => e.status === 404);

    assert.strictEqual(await exec('bragi.sessions.open', 'c-old'), 3 * 2 - 2);
    assert.strictEqual(api.chat.controller.conversationId, 'c-old');
  });

  test('chats are filed in folders, which nest, rename, move and delete without losing a chat', async () => {
    mock.chats.set('c-1', [{ id: 'u1', role: 'user', text: 'Hi', state: 'complete' }]);
    const work = await exec<{ id: string }>('bragi.folders.new', 'Work');
    const sub = await exec<{ id: string }>('bragi.folders.new', 'Bragi', work.id);
    await exec('bragi.sessions.moveToFolder', 'c-1', sub.id);
    assert.strictEqual(mock.sessionFolders.get('c-1'), sub.id);
    await exec('bragi.folders.rename', sub.id, 'Bragi code');
    assert.deepStrictEqual([...folderPaths(mock.folders).values()].sort(), ['Work', 'Work / Bragi code']);
    const items = sessionItems(
      [
        { id: 'c-1', title: 'Hi', updated_ms: 2, messages: 1, target_label: 'Weirdnet pool', folder: sub.id },
        { id: 's-1', title: 'First chat', updated_ms: 1, messages: 2, target_label: 'Weirdnet pool' },
      ],
      mock.folders,
    );
    assert.deepStrictEqual(
      items.map((i) => (i.kind === vscode.QuickPickItemKind.Separator ? `# ${i.label}` : i.label)),
      ['# No folder', 'First chat', '# Work / Bragi code', 'Hi'],
    );
    await exec('bragi.folders.move', sub.id, null);
    assert.strictEqual(mock.folders.find((f) => f.id === sub.id)!.parent, null);
    await exec('bragi.folders.move', sub.id, work.id);
    await exec('bragi.folders.delete', sub.id);
    assert.strictEqual(mock.sessionFolders.get('c-1'), work.id, 'the chat moves up a level');
    assert.ok(mock.chats.has('c-1'));
    await exec('bragi.sessions.moveToFolder', 'c-1', null);
    assert.strictEqual(mock.sessionFolders.has('c-1'), false);
  });

  test('a link code is shown for another device, compared and confirmed', async () => {
    mock.pair = { state: 'unpaired' };
    mock.offerScript = 'match';
    const done = await exec<{ state: string }>('bragi.link.offer', true);
    assert.strictEqual(done.state, 'linked');
    assert.strictEqual(await vscode.env.clipboard.readText(), 'LINK-CODE-42', 'the code is copied to paste on the other device');

    mock.pair = { state: 'unpaired' };
    const no = await exec<{ state: string; message?: string }>('bragi.link.offer', false);
    assert.deepStrictEqual(no, { state: 'failed', message: 'the codes did not match' });
    mock.pair = { state: 'linked', user: 'u', device: 'd' };
  });

  test('the direct target takes text files inline and refuses images', async () => {
    await set('target', 'direct');
    await set('direct.url', `${mock.url}/openai/v1`);
    try {
      const c = new ChatController(api.router, () => undefined);
      assert.throws(() => c.attachFile(file('shot.png', PNG)), /need the Bragi target/);
      c.attachFile(file('notes.md', 'add sums'));
      assert.strictEqual(await c.send('summarise'), 'Hello from direct');
      const sent = mock.last('/openai/v1/chat/completions')!.body.messages.pop().content as string;
      assert.ok(sent.startsWith('notes.md:\n```\nadd sums\n```\n') && sent.endsWith('summarise'), sent);
    } finally {
      await set('direct.url', '');
      await set('target', 'bragi');
    }
  });

  test('the commands are there', async () => {
    const all = await vscode.commands.getCommands(true);
    for (const id of ['bragi.chat.attachFile', 'bragi.chat.attachSelection', 'bragi.sessions.open', 'bragi.sessions.moveToFolder', 'bragi.folders.new', 'bragi.folders.rename', 'bragi.folders.move', 'bragi.folders.delete', 'bragi.link.offer']) {
      assert.ok(all.includes(id), id);
    }
  });
});
