// Keeps `bragi serve` running for the extension: finds `bragi`, starts it detached when nothing
// answers, waits until it does, and reports its state for the status bar. It never stops a
// server it didn't start, and a server it starts stops itself after 30 idle minutes.

import * as childProcess from 'child_process';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as vscode from 'vscode';
import { Secrets, defaultTokenPath } from './tokens';

export type ServeKind = 'unknown' | 'starting' | 'running' | 'stopped' | 'failed';

export interface ServeState {
  kind: ServeKind;
  /** Running: this extension started it. */
  started?: boolean;
  /** Failed: why. */
  message?: string;
  /** Failed because no `bragi` was found. */
  notFound?: boolean;
}

export interface Spawnable {
  program: string;
  args: string[];
}

export const IDLE_MINUTES = 30;
const STARTED_PID = 'bragi.serve.startedPid';
const LOCK_STALE_MS = 60_000;

const exeNames = (base: string, platform: NodeJS.Platform): string[] => (platform === 'win32' ? [`${base}.exe`, `${base}.cmd`] : [base]);

/** The `bragi` to run: the `bragi.path` setting, else `bragi` (then `bragi-serve`) on the PATH. */
export function findBragi(configured: string, env: NodeJS.ProcessEnv = process.env, platform: NodeJS.Platform = process.platform): Spawnable | undefined {
  const isFile = (p: string) => {
    try {
      return fs.statSync(p).isFile();
    } catch {
      return false;
    }
  };
  const forName = (p: string): Spawnable => ({ program: p, args: path.basename(p).replace(/\.(exe|cmd)$/i, '') === 'bragi-serve' ? [] : ['serve'] });
  if (configured.trim()) {
    const p = configured.trim().replace(/^~(?=$|[\\/])/, os.homedir());
    return isFile(p) ? forName(p) : undefined;
  }
  const dirs = (env.PATH ?? env.Path ?? '').split(platform === 'win32' ? ';' : ':').filter(Boolean);
  for (const base of ['bragi', 'bragi-serve']) {
    for (const d of dirs) {
      for (const n of exeNames(base, platform)) {
        const p = path.join(d, n);
        if (isFile(p)) {
          return forName(p);
        }
      }
    }
  }
  return undefined;
}

/** The port of a `bragi.serveUrl`. */
export function portOf(url: string): number {
  try {
    const u = new URL(url);
    return Number(u.port) || (u.protocol === 'https:' ? 443 : 80);
  } catch {
    return 4877;
  }
}

/** The Bragi home a token file belongs to (`<home>/serve/token`). */
export function homeOfToken(tokenFile: string): string {
  return path.dirname(path.dirname(tokenFile));
}

/** The arguments after the program: where it keeps its files, its port, and the idle stop. */
export function serveArgs(base: string[], home: string, port: number, idleMinutes = IDLE_MINUTES): string[] {
  const a = [...base, '--home', home, '--port', String(port)];
  if (idleMinutes > 0) {
    a.push('--idle-timeout', String(idleMinutes));
  }
  return a;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

export class ServeManager implements vscode.Disposable {
  private current: ServeState = { kind: 'unknown' };
  private inflight?: Promise<ServeState>;
  private readonly changed = new vscode.EventEmitter<ServeState>();
  readonly onDidChange = this.changed.event;
  private warned = false;

  constructor(
    private readonly secrets: Secrets,
    private readonly memento: vscode.Memento,
    /** Whether requests go to the local Bragi API (the `bragi` target), not a direct server. */
    private readonly wantsBragi: () => boolean = () => true,
    private readonly timeoutMs = 25_000,
  ) {}

  dispose(): void {
    this.changed.dispose();
  }

  get state(): ServeState {
    return this.current;
  }

  private cfg() {
    return vscode.workspace.getConfiguration('bragi');
  }

  private set(s: ServeState): ServeState {
    const same = s.kind === this.current.kind && s.started === this.current.started && s.message === this.current.message;
    this.current = s;
    if (!same) {
      this.changed.fire(s);
    }
    return s;
  }

  get port(): number {
    return portOf(this.cfg().get('serveUrl', 'http://127.0.0.1:4877'));
  }

  get tokenFile(): string {
    return defaultTokenPath();
  }

  private async healthy(): Promise<boolean> {
    try {
      const res = await fetch(`http://127.0.0.1:${this.port}/v1/health`, { signal: AbortSignal.timeout(1500) });
      if (!res.ok) {
        return false;
      }
      const body = (await res.json()) as { ok?: boolean; version?: string };
      return body.ok === true && typeof body.version === 'string';
    } catch {
      return false;
    }
  }

  /** Whether the token file's token is accepted; undefined when there is no file to try. */
  private async tokenAccepted(): Promise<boolean | undefined> {
    let token: string;
    try {
      token = (await fs.promises.readFile(this.tokenFile, 'utf8')).trim();
    } catch {
      return undefined;
    }
    try {
      const res = await fetch(`http://127.0.0.1:${this.port}/v1/state`, { headers: { authorization: `Bearer ${token}` }, signal: AbortSignal.timeout(3000) });
      return res.status !== 401;
    } catch {
      return undefined;
    }
  }

  /** The pid in `server.pid` (written by the running server). */
  private serverPid(): number | undefined {
    try {
      const n = Number(fs.readFileSync(path.join(homeOfToken(this.tokenFile), 'serve', 'server.pid'), 'utf8').split('\n')[0]);
      return Number.isInteger(n) && n > 0 ? n : undefined;
    } catch {
      return undefined;
    }
  }

  /** Looks at the port without starting anything. */
  async check(): Promise<ServeState> {
    if (this.current.kind === 'starting' && this.inflight) {
      return this.current;
    }
    if (!(await this.healthy())) {
      const was = this.current.kind;
      return this.set(was === 'running' || was === 'stopped' ? { kind: 'stopped' } : was === 'failed' ? this.current : { kind: 'stopped' });
    }
    if ((await this.tokenAccepted()) === false) {
      return this.set({ kind: 'failed', message: `A Bragi server on port ${this.port} doesn't accept this Bragi home's token (it uses another home).` });
    }
    const pid = this.serverPid();
    const ours = pid !== undefined && this.memento.get<number>(STARTED_PID) === pid;
    return this.set({ kind: 'running', started: ours });
  }

  /** Makes sure a server is answering, starting one when none is and the settings allow it. */
  ensure(opts: { force?: boolean } = {}): Promise<ServeState> {
    this.inflight ??= this.doEnsure(opts.force === true).finally(() => {
      this.inflight = undefined;
    });
    return this.inflight;
  }

  private async doEnsure(force: boolean): Promise<ServeState> {
    const up = await this.check();
    if (up.kind === 'running') {
      await this.importToken();
      return up;
    }
    if (up.kind === 'failed') {
      return up;
    }
    const allowed = force || (this.cfg().get('autoStartServer', true) && this.wantsBragi() && vscode.workspace.isTrusted);
    if (!allowed) {
      return this.set({ kind: 'stopped' });
    }
    const bragi = findBragi(this.cfg().get('path', ''));
    if (!bragi) {
      const configured = this.cfg().get<string>('path', '').trim();
      const message = configured
        ? `bragi.path (${configured}) isn't a file.`
        : "Couldn't find `bragi` on the PATH to start the server. Install Bragi, or set bragi.path to the program.";
      this.tell(message);
      return this.set({ kind: 'failed', message, notFound: true });
    }
    this.set({ kind: 'starting' });
    try {
      return await this.start(bragi);
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      this.tell(`Bragi's server didn't start: ${message}`);
      return this.set({ kind: 'failed', message });
    }
  }

  private async start(bragi: Spawnable): Promise<ServeState> {
    const home = homeOfToken(this.tokenFile);
    const dir = path.join(home, 'serve');
    await fs.promises.mkdir(dir, { recursive: true });
    const lock = path.join(dir, 'start.lock');
    const until = Date.now() + this.timeoutMs;
    let held = false;
    while (!held) {
      try {
        await fs.promises.writeFile(lock, String(process.pid), { flag: 'wx' });
        held = true;
      } catch {
        // Another starter (the terminal app, another window): wait for its server.
        const age = await fs.promises.stat(lock).then((s) => Date.now() - s.mtimeMs, () => 0);
        if (age > LOCK_STALE_MS) {
          await fs.promises.rm(lock, { force: true });
        } else if (await this.healthy()) {
          return this.finish(false);
        } else if (Date.now() > until) {
          throw new Error('another process is starting the server and it is not answering');
        } else {
          await sleep(150);
        }
      }
    }
    let child: childProcess.ChildProcess | undefined;
    try {
      if (await this.healthy()) {
        return this.finish(false);
      }
      const log = fs.openSync(path.join(dir, 'server.log'), 'a');
      let exited: string | undefined;
      try {
        child = childProcess.spawn(bragi.program, serveArgs(bragi.args, home, this.port), {
          detached: true,
          stdio: ['ignore', log, log],
          windowsHide: true,
        });
      } finally {
        fs.closeSync(log);
      }
      child.once('error', (e) => (exited = `couldn't run ${bragi.program}: ${e.message}`));
      child.once('exit', (code) => (exited = `the server exited (code ${code}); see ${path.join(dir, 'server.log')}`));
      child.unref();
      while (!(await this.healthy())) {
        if (exited) {
          throw new Error(exited);
        }
        if (Date.now() > until) {
          throw new Error(`the server didn't answer in time; see ${path.join(dir, 'server.log')}`);
        }
        await sleep(100);
      }
      await this.memento.update(STARTED_PID, child.pid);
      return this.finish(true);
    } finally {
      await fs.promises.rm(lock, { force: true });
    }
  }

  private async finish(started: boolean): Promise<ServeState> {
    await this.importToken();
    return this.set({ kind: 'running', started });
  }

  /** The token file is the truth when there is one: keep SecretStorage in step with it. */
  private async importToken(): Promise<void> {
    let fileToken: string | undefined;
    try {
      fileToken = (await fs.promises.readFile(this.tokenFile, 'utf8')).trim();
    } catch {
      return;
    }
    if (fileToken.length >= 32 && (await this.secrets.serveToken()) !== fileToken) {
      await this.secrets.setServeToken(fileToken);
    }
  }

  private tell(message: string): void {
    if (this.warned) {
      return;
    }
    this.warned = true;
    // Not awaited: nothing waits on the user to close a notification.
    void vscode.window.showErrorMessage(`Bragi: ${message}`, 'Open Settings').then((pick) => {
      if (pick === 'Open Settings') {
        void vscode.commands.executeCommand('workbench.action.openSettings', 'bragi.path');
      }
    });
  }

  /** "Bragi: Stop Server": stops one this extension started; leaves another's alone. */
  async stop(): Promise<ServeState> {
    const now = await this.check();
    if (now.kind !== 'running') {
      return now;
    }
    const pid = this.serverPid();
    if (!now.started || pid === undefined) {
      void vscode.window.showInformationMessage("Bragi's server wasn't started by this editor, so it is left running.");
      return now;
    }
    try {
      process.kill(pid);
    } catch (e) {
      void vscode.window.showWarningMessage(`Bragi: couldn't stop the server: ${e instanceof Error ? e.message : String(e)}`);
      return now;
    }
    const until = Date.now() + 8000;
    while ((await this.healthy()) && Date.now() < until) {
      await sleep(100);
    }
    return this.set({ kind: 'stopped' });
  }

  /** "Bragi: Start/Restart Server": starts one; restarts one this extension started; leaves another's alone. */
  async restart(): Promise<ServeState> {
    this.warned = false;
    const now = await this.check();
    if (now.kind === 'running') {
      const pid = this.serverPid();
      if (!now.started || pid === undefined) {
        void vscode.window.showInformationMessage("Bragi's server is already running and wasn't started by this editor, so it is left as it is.");
        return now;
      }
      try {
        process.kill(pid);
      } catch (e) {
        void vscode.window.showWarningMessage(`Bragi: couldn't stop the server: ${e instanceof Error ? e.message : String(e)}`);
        return now;
      }
      const until = Date.now() + 8000;
      while ((await this.healthy()) && Date.now() < until) {
        await sleep(100);
      }
    } else if (now.kind === 'failed' && !now.notFound) {
      return now;
    }
    this.current = { kind: 'unknown' };
    return this.ensure({ force: true });
  }
}

/** The status-bar text, tooltip and warning colour for a state. */
export function describeServe(s: ServeState, port: number): { text: string; tooltip: string; warn: boolean } {
  switch (s.kind) {
    case 'running':
      return { text: '$(server) Bragi', tooltip: `Bragi's server is running on port ${port}${s.started ? ' (started by this editor; it stops itself after 30 idle minutes)' : ''}. Click to start or restart it.`, warn: false };
    case 'starting':
      return { text: '$(sync~spin) Bragi', tooltip: "Starting Bragi's server…", warn: false };
    case 'failed':
      return { text: '$(error) Bragi', tooltip: `${s.message ?? 'The server is not running.'} Click to try again.`, warn: true };
    case 'stopped':
      return { text: '$(circle-slash) Bragi', tooltip: "Bragi's server isn't running. Click to start it.", warn: true };
    default:
      return { text: '$(server) Bragi', tooltip: 'Checking Bragi’s server…', warn: false };
  }
}
