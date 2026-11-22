// The Bragi API token and the direct endpoint's key, kept in the editor's SecretStorage only.

import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import * as vscode from 'vscode';

const SERVE_TOKEN = 'bragi.serveToken';
const DIRECT_KEY = 'bragi.directKey';

/** Where `bragi serve` keeps its token: like the app's home (BRAGI_HOME, XDG_DATA_HOME, ~/.local/share). */
export function defaultTokenPath(env: NodeJS.ProcessEnv = process.env): string {
  const configured = vscode.workspace.getConfiguration('bragi').get<string>('tokenFile', '');
  if (configured) {
    return configured.replace(/^~(?=$|[\\/])/, os.homedir());
  }
  let home = env.BRAGI_HOME;
  if (!home) {
    const base = env.XDG_DATA_HOME || (env.HOME ? path.join(env.HOME, '.local', 'share') : env.APPDATA || '.');
    home = path.join(base, 'bragi');
  }
  return path.join(home, 'serve', 'token');
}

export class Secrets {
  constructor(private readonly store: vscode.SecretStorage) {}

  async serveToken(): Promise<string | undefined> {
    return (await this.store.get(SERVE_TOKEN)) || undefined;
  }

  async setServeToken(token: string): Promise<void> {
    await this.store.store(SERVE_TOKEN, token.trim());
  }

  async clearServeToken(): Promise<void> {
    await this.store.delete(SERVE_TOKEN);
  }

  /** Reads the token file `bragi serve` made and keeps it in SecretStorage. False when there is none. */
  async importServeToken(file = defaultTokenPath()): Promise<boolean> {
    let token: string;
    try {
      token = (await fs.promises.readFile(file, 'utf8')).trim();
    } catch {
      return false;
    }
    if (token.length < 32) {
      return false;
    }
    await this.setServeToken(token);
    return true;
  }

  async directKey(): Promise<string | undefined> {
    return (await this.store.get(DIRECT_KEY)) || undefined;
  }

  async setDirectKey(key: string): Promise<void> {
    if (key.trim()) {
      await this.store.store(DIRECT_KEY, key.trim());
    } else {
      await this.store.delete(DIRECT_KEY);
    }
  }
}
