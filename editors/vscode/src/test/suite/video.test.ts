// Video in the chat panel (LF-43, V2): which files are clips, the player message and its fallbacks,
// the webview's resource roots, drops, and the chat controller against a mock `bragi serve`
// (attach, the limits chip and override, the clip going with the turn, the `video` event).

import * as assert from 'assert';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as vscode from 'vscode';
import type { BragiApi } from '../../extension';
import { ChatController, ViewMessage } from '../../chat';
import { BragiError } from '../../types';
import { droppedClip, fallbackMessage, isVideoPath, playerMessage, resourceRoots, VideoEvent, webviewType, withSource } from '../../video';
import { MockServer, TOKEN } from './mockServer';

const cfg = () => vscode.workspace.getConfiguration('bragi');
const set = (key: string, value: unknown) => cfg().update(key, value, vscode.ConfigurationTarget.Global);

const event = (clip: string | null): VideoEvent => ({
  type: 'video',
  name: 'clip.mp4',
  bubble: 'video  clip.mp4  0:42  2 frames  no sound sent',
  summary: 'Video "clip.mp4", 00:42, 2 frames sampled.',
  duration_ms: 42000,
  sound: false,
  clip,
  frames: [
    { at_ms: 10500, label: '[00:10.5]', mime: 'image/jpeg', data: '/9j/AA==' },
    { at_ms: 31500, label: '[00:31.5]', mime: 'text/html', data: 'PHNjcmlwdD4=' },
  ],
});

suite('Video in the chat', () => {
  const mock = new MockServer();
  let api: BragiApi;
  let dir: string;
  let clip: string;

  suiteSetup(async () => {
    await mock.start();
    const ext = vscode.extensions.getExtension<BragiApi>('weirdbrains.bragi');
    api = await ext!.activate();
    await set('serveUrl', mock.url);
    await set('target', 'bragi');
    await set('autoStartServer', false);
    await vscode.commands.executeCommand('bragi.setToken', TOKEN);
    dir = fs.mkdtempSync(path.join(process.env.BRAGI_TEST_TMP || os.tmpdir(), 'clips-'));
    clip = path.join(dir, 'clip.mp4');
    fs.writeFileSync(clip, 'not a real clip');
  });

  suiteTeardown(async () => {
    await mock.stop();
    fs.rmSync(dir, { recursive: true, force: true });
  });

  test('clips are known by extension, and only some play in the webview', () => {
    assert.ok(isVideoPath('/a/b/Clip.MP4'));
    assert.ok(isVideoPath('x.webm') && isVideoPath('x.mkv'));
    assert.ok(!isVideoPath('notes.txt') && !isVideoPath('mp4'));
    assert.strictEqual(webviewType('a.mp4'), 'video/mp4');
    assert.strictEqual(webviewType('a.webm'), 'video/webm');
    assert.strictEqual(webviewType('a.mkv'), undefined);
    assert.strictEqual(webviewType('a.avi'), undefined);
  });

  test('the player message plays a clip on disk, else says why', () => {
    const here = playerMessage(event(clip), () => true);
    assert.strictEqual(here.clip, clip);
    assert.strictEqual(here.mime, 'video/mp4');
    assert.strictEqual(here.fallback, undefined);
    // Only image frames, as data: URLs; anything else is dropped.
    assert.deepStrictEqual(here.frames, [{ at_ms: 10500, label: '[00:10.5]', src: 'data:image/jpeg;base64,/9j/AA==' }]);
    assert.strictEqual(withSource(here, (p) => `vscode-resource:${p}`).src, `vscode-resource:${clip}`);

    const gone = playerMessage(event(clip), () => false);
    assert.strictEqual(gone.fallback, 'missing');
    assert.strictEqual(gone.clip, undefined, 'no system player for a file that is not there');
    assert.strictEqual(withSource(gone, () => 'x').src, undefined);
    assert.strictEqual(playerMessage(event(null), () => true).fallback, 'missing');
    assert.strictEqual(playerMessage(event('relative/clip.mp4'), () => true).fallback, 'missing');

    const mkv = playerMessage(event(path.join(dir, 'clip.mkv')), () => true);
    assert.strictEqual(mkv.fallback, 'format');
    assert.ok(mkv.clip, 'the system player can still open it');
    assert.strictEqual(withSource(mkv, () => 'x').src, undefined);
    assert.match(fallbackMessage('format'), /system player/);
    assert.match(fallbackMessage('missing'), /isn't on this machine/);
    assert.match(fallbackMessage('error'), /codec/);
  });

  test('the webview reads only its media and the played clips’ folders', () => {
    const roots = resourceRoots('/ext/media', ['/v/a.mp4', '/v/b.webm', '/w/c.mp4']);
    assert.deepStrictEqual(roots, ['/ext/media', '/v', '/w']);
  });

  test('a drop names a local clip', () => {
    assert.strictEqual(droppedClip(vscode.Uri.file(clip).toString()), vscode.Uri.file(clip).fsPath);
    assert.strictEqual(droppedClip(`# comment\r\nhttps://example.com/a.mp4\r\n${vscode.Uri.file(clip).toString()}`), vscode.Uri.file(clip).fsPath);
    assert.strictEqual(droppedClip(vscode.Uri.file(path.join(dir, 'notes.txt')).toString()), undefined);
    assert.strictEqual(droppedClip(''), undefined);
  });

  test('an attached clip shows the limits chip, the override sets them, and the clip goes with the turn', async () => {
    const seen: ViewMessage[] = [];
    const c = new ChatController(api.router, (m) => seen.push(m));
    mock.videoUser = '';

    await assert.rejects(c.attachVideo(path.join(dir, 'notes.txt')), (e: BragiError) => /isn't a video/.test(e.message));
    await assert.rejects(c.attachVideo(path.join(dir, 'gone.mp4')), (e: BragiError) => /isn't there/.test(e.message));

    await c.attachVideo(clip);
    assert.strictEqual(c.pendingVideo, clip);
    const chip = seen.filter((m) => m.type === 'attached').pop() as { chip?: string; ready?: boolean };
    assert.match(chip.chip!, /needs image cost/);
    assert.strictEqual(chip.ready, false);

    await assert.rejects(c.setVideoLimits('frames=3'), (e: BragiError) => e.kind === 'bad_request');
    const after = await c.setVideoLimits('image_tokens=256 context=99999999');
    assert.strictEqual(after.ready, true);
    assert.deepStrictEqual(after.ignored, ['context']);
    assert.strictEqual(mock.last('/v1/video/limits/set')!.body.model, 'gemma-4-e4b');
    assert.match((seen.filter((m) => m.type === 'attached').pop() as { chip: string }).chip, /video ready/);

    const text = await c.send('What happens in it?');
    assert.strictEqual(text, 'It adds two numbers.');
    assert.deepStrictEqual(mock.last('/v1/code/chat')!.body.video, { path: clip });
    assert.strictEqual(c.pendingVideo, undefined, 'the clip goes once');
    assert.ok(seen.some((m) => m.type === 'detached'));
    const user = seen.find((m) => m.type === 'user') as { context?: string };
    assert.strictEqual(user.context, 'clip.mp4');
    const player = seen.find((m) => m.type === 'video') as { clip?: string; mime?: string; frames: unknown[]; bubble: string };
    assert.strictEqual(player.clip, clip);
    assert.strictEqual(player.mime, 'video/mp4');
    assert.strictEqual(player.frames.length, 2);
    assert.match(player.bubble, /^video {2}clip\.mp4/);
    assert.ok(c.clips.has(clip), 'only clips the chat showed open in the system player');

    // The next turn has no clip.
    await c.send('And then?');
    assert.strictEqual(mock.last('/v1/code/chat')!.body.video, undefined);
    c.detachVideo();
  });

  test('the direct target refuses a clip', async () => {
    const c = new ChatController(api.router, () => undefined);
    await set('target', 'direct');
    try {
      await assert.rejects(c.attachVideo(clip), (e: BragiError) => /Bragi target/.test(e.message));
      await assert.rejects(api.router.direct.chat({ prompt: 'x', video: { path: clip } }, () => undefined), (e: BragiError) => /Bragi target/.test(e.message));
    } finally {
      await set('target', 'bragi');
    }
  });

  test('its commands are there', async () => {
    const all = await vscode.commands.getCommands(true);
    assert.ok(all.includes('bragi.attachVideo') && all.includes('bragi.videoLimits'));
    const pkg = JSON.parse(fs.readFileSync(path.join(__dirname, '../../../package.json'), 'utf8'));
    const menu = pkg.contributes.menus['explorer/context'].find((m: { command: string }) => m.command === 'bragi.attachVideo');
    assert.match(menu.when, /mp4/);
  });
});
