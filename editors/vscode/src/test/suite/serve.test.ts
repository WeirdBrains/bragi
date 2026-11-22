import * as assert from 'assert';
import * as fs from 'fs';
import * as http from 'http';
import * as net from 'net';
import * as os from 'os';
import * as path from 'path';
import * as vscode from 'vscode';
import type { BragiApi } from '../../extension';
import { describeServe, findBragi, homeOfToken, portOf, serveArgs, ServeManager } from '../../serve';
import { Secrets } from '../../tokens';

const cfg = () => vscode.workspace.getConfiguration('bragi');
const set = (key: string, value: unknown) => cfg().update(key, value, vscode.ConfigurationTarget.Global);

class MemoryMemento implements vscode.Memento {
  private data = new Map<string, unknown>();
  keys(): readonly string[] {
    return [...this.data.keys()];
  }
  get<T>(key: string, fallback?: T): T | undefined {
    return (this.data.has(key) ? this.data.get(key) : fallback) as T | undefined;
  }
  async update(key: string, value: unknown): Promise<void> {
    this.data.set(key, value);
  }
}

class MemorySecrets implements vscode.SecretStorage {
  private data = new Map<string, string>();
  readonly onDidChange = new vscode.EventEmitter<vscode.SecretStorageChangeEvent>().event;
  async get(k: string) {
    return this.data.get(k);
  }
  async store(k: string, v: string) {
    this.data.set(k, v);
  }
  async delete(k: string) {
    this.data.delete(k);
  }
  keys() {
    return Promise.resolve([...this.data.keys()]);
  }
}

async function freePort(): Promise<number> {
  return new Promise((resolve) => {
    const s = net.createServer();
    s.listen(0, '127.0.0.1', () => {
      const p = (s.address() as net.AddressInfo).port;
      s.close(() => resolve(p));
    });
  });
}

// A stand-in `bragi`: `bragi serve --home H --port P --idle-timeout N` keeps a token file, a pid
// file and a loopback server, and logs each start.
const FAKE_JS = `
const http = require('http'), fs = require('fs'), path = require('path');
const a = process.argv.slice(2);
const arg = (n) => a[a.indexOf(n) + 1];
if (a[0] !== 'serve') { process.exit(2); }
const home = arg('--home'), port = Number(arg('--port'));
const dir = path.join(home, 'serve');
fs.mkdirSync(dir, { recursive: true });
const tf = path.join(dir, 'token');
if (!fs.existsSync(tf)) fs.writeFileSync(tf, 'a'.repeat(64), { mode: 0o600 });
const token = fs.readFileSync(tf, 'utf8').trim();
fs.appendFileSync(path.join(home, 'started.log'), process.pid + ' ' + a.join(' ') + '\\n');
http.createServer((req, res) => {
  res.setHeader('content-type', 'application/json');
  if (req.url === '/v1/health') return res.end(JSON.stringify({ ok: true, version: 'fake' }));
  if (req.headers.authorization !== 'Bearer ' + token) { res.statusCode = 401; return res.end('{}'); }
  res.end(JSON.stringify({ linked: true, models: [] }));
}).listen(port, '127.0.0.1', () => fs.writeFileSync(path.join(dir, 'server.pid'), process.pid + '\\n' + port + '\\n'));
process.on('SIGTERM', () => { try { fs.unlinkSync(path.join(dir, 'server.pid')); } catch {} process.exit(0); });
`;

suite('Bragi server autostart', function () {
  const unix = process.platform !== 'win32';
  let scratch: string;
  let home: string;
  let fake: string;
  let port: number;
  const secrets = new MemorySecrets();
  const mk = () => new ServeManager(new Secrets(secrets), new MemoryMemento(), () => true, 15_000);
  const startedLog = () => (fs.existsSync(path.join(home, 'started.log')) ? fs.readFileSync(path.join(home, 'started.log'), 'utf8').trim().split('\n').filter(Boolean) : []);

  const killOurs = () => {
    try {
      const pid = Number(fs.readFileSync(path.join(home, 'serve', 'server.pid'), 'utf8').split('\n')[0]);
      if (pid) {
        process.kill(pid);
      }
    } catch {
      // not running
    }
  };
  const until = async (what: string, f: () => Promise<boolean> | boolean) => {
    const end = Date.now() + 8000;
    while (!(await f())) {
      assert.ok(Date.now() < end, `waiting for ${what}`);
      await new Promise((r) => setTimeout(r, 50));
    }
  };

  setup(async () => {
    scratch = fs.mkdtempSync(path.join(process.env.BRAGI_TEST_TMP || os.tmpdir(), 'bragi-serve-'));
    home = path.join(scratch, 'home');
    fs.mkdirSync(home, { recursive: true });
    fake = path.join(scratch, 'bragi');
    fs.writeFileSync(path.join(scratch, 'fake.js'), FAKE_JS);
    fs.writeFileSync(fake, `#!/bin/sh\nexec "$BRAGI_TEST_NODE" "${path.join(scratch, 'fake.js')}" "$@"\n`, { mode: 0o755 });
    port = await freePort();
    await set('serveUrl', `http://127.0.0.1:${port}`);
    await set('tokenFile', path.join(home, 'serve', 'token'));
    await set('path', fake);
    await set('autoStartServer', true);
    await set('target', 'bragi');
  });

  teardown(async () => {
    killOurs();
    await set('path', undefined);
    await set('tokenFile', undefined);
    await set('autoStartServer', false);
    fs.rmSync(scratch, { recursive: true, force: true });
  });

  test('the program and its arguments are found and built', () => {
    const bin = path.join(scratch, 'pathdir');
    fs.mkdirSync(bin);
    fs.writeFileSync(path.join(bin, 'bragi-serve'), '#!/bin/sh\n', { mode: 0o755 });
    const found = findBragi('', { PATH: bin }, 'linux');
    assert.deepStrictEqual(found, { program: path.join(bin, 'bragi-serve'), args: [] });
    fs.writeFileSync(path.join(bin, 'bragi'), '#!/bin/sh\n', { mode: 0o755 });
    assert.deepStrictEqual(findBragi('', { PATH: bin }, 'linux'), { program: path.join(bin, 'bragi'), args: ['serve'] });
    assert.strictEqual(findBragi('', { PATH: '' }, 'linux'), undefined);
    assert.strictEqual(findBragi(path.join(scratch, 'missing')), undefined);
    assert.deepStrictEqual(findBragi(path.join(bin, 'bragi')), { program: path.join(bin, 'bragi'), args: ['serve'] });
    // Windows names.
    const win = path.join(scratch, 'windir');
    fs.mkdirSync(win);
    fs.writeFileSync(path.join(win, 'bragi.exe'), '');
    assert.strictEqual(findBragi('', { PATH: win }, 'win32')?.program, path.join(win, 'bragi.exe'));
    assert.strictEqual(portOf('http://127.0.0.1:5000'), 5000);
    assert.strictEqual(homeOfToken('/h/bragi/serve/token'), '/h/bragi');
    assert.deepStrictEqual(serveArgs(['serve'], '/h', 4877), ['serve', '--home', '/h', '--port', '4877', '--idle-timeout', '30']);
  });

  test('it starts the server once, reuses it, and keeps the token in step', async function () {
    if (!unix) {
      return this.skip();
    }
    const a = mk();
    const [x, y] = await Promise.all([a.ensure(), a.ensure()]);
    assert.strictEqual(x.kind, 'running');
    assert.strictEqual(y, x, 'both callers share one start');
    assert.strictEqual(x.started, true);
    assert.strictEqual(startedLog().length, 1);
    assert.match(startedLog()[0], /serve --home .* --port \d+ --idle-timeout 30$/);
    assert.strictEqual(await new Secrets(secrets).serveToken(), 'a'.repeat(64));
    // A second manager (another window) reuses the same server.
    const b = mk();
    assert.strictEqual((await b.ensure()).kind, 'running');
    assert.strictEqual(startedLog().length, 1, 'nothing started twice');
    assert.ok(!fs.existsSync(path.join(home, 'serve', 'start.lock')), 'the lock is released');
    assert.ok(describeServe(x, port).tooltip.includes(String(port)));
  });

  test('autoStartServer off starts nothing', async function () {
    if (!unix) {
      return this.skip();
    }
    await set('autoStartServer', false);
    const s = await mk().ensure();
    assert.strictEqual(s.kind, 'stopped');
    assert.strictEqual(startedLog().length, 0);
    assert.ok(describeServe(s, port).warn);
  });

  test('a missing program is a clear failure, not a hang', async () => {
    await set('path', path.join(scratch, 'no-such-bragi'));
    const s = await mk().ensure();
    assert.strictEqual(s.kind, 'failed');
    assert.strictEqual(s.notFound, true);
    assert.match(s.message ?? '', /bragi\.path .* isn't a file/);
    await set('path', '');
    const saved = process.env.PATH;
    process.env.PATH = scratch + '/empty';
    try {
      const t = await mk().ensure();
      assert.match(t.message ?? '', /Couldn't find `bragi` on the PATH/);
    } finally {
      process.env.PATH = saved;
    }
  });

  test('a server it did not start is reused and left alone', async function () {
    if (!unix) {
      return this.skip();
    }
    const other = http.createServer((req, res) => {
      res.setHeader('content-type', 'application/json');
      res.end(req.url === '/v1/health' ? JSON.stringify({ ok: true, version: 'elsewhere' }) : '{}');
    });
    await new Promise<void>((r) => other.listen(port, '127.0.0.1', r));
    try {
      const m = mk();
      const s = await m.ensure();
      assert.strictEqual(s.kind, 'running');
      assert.strictEqual(s.started, false);
      assert.strictEqual(startedLog().length, 0);
      const again = await m.restart();
      assert.strictEqual(again.kind, 'running');
      assert.strictEqual(startedLog().length, 0, 'restart does not replace a server it did not start');
      await fetch(`http://127.0.0.1:${port}/v1/health`).then((r) => assert.ok(r.ok), () => assert.fail('the other server is still up'));
    } finally {
      await new Promise((r) => other.close(r));
    }
  });

  test('restart replaces a server this editor started', async function () {
    if (!unix) {
      return this.skip();
    }
    const m = mk();
    await m.ensure();
    const first = startedLog();
    assert.strictEqual(first.length, 1);
    // The next check knows it by its pid.
    assert.strictEqual((await m.check()).started, true);
    const s = await m.restart();
    assert.strictEqual(s.kind, 'running');
    await until('the second start', () => startedLog().length === 2);
    assert.notStrictEqual(startedLog()[0].split(' ')[0], startedLog()[1].split(' ')[0]);
  });

  test('a request revives a server that stopped', async function () {
    if (!unix) {
      return this.skip();
    }
    const ext = vscode.extensions.getExtension<BragiApi>('weirdbrains.bragi');
    const api = await ext!.activate();
    const before = startedLog().length;
    await api.serve.ensure();
    await until('the server', async () => (await api.serve.check()).kind === 'running');
    assert.ok(startedLog().length >= before + 1);
    killOurs();
    await until('it to stop', async () => (await api.serve.check()).kind === 'stopped');
    // Asking for the models starts it again (the backend's revive hook).
    const list = await api.router.bragi.models();
    assert.ok(Array.isArray(list.models));
    assert.strictEqual(api.serve.state.kind, 'running');
    assert.ok((await vscode.commands.getCommands(true)).includes('bragi.restartServer'));
  });
});
