// Where each setting's value comes from. Precedence, highest first: workspace folder, workspace,
// user (global), then the default, which is Bragi's own. VS Code resolves that through
// `workspace.getConfiguration`; this file only explains it and builds the `bragi mcp` command line.
// Keys that run a program or point at a server are `machine` scope: a workspace can't set them.

import * as vscode from 'vscode';

export type Source = 'workspace folder' | 'workspace' | 'user' | 'default';

export interface Inspected<T> {
  key: string;
  defaultValue?: T;
  globalValue?: T;
  workspaceValue?: T;
  workspaceFolderValue?: T;
}

/** Which scope wins for an inspected setting. */
export function sourceOf<T>(i: Inspected<T> | undefined): Source {
  if (i?.workspaceFolderValue !== undefined) {
    return 'workspace folder';
  }
  if (i?.workspaceValue !== undefined) {
    return 'workspace';
  }
  if (i?.globalValue !== undefined) {
    return 'user';
  }
  return 'default';
}

/** The effective value of an inspected setting, by the same order. */
export function effectiveOf<T>(i: Inspected<T> | undefined): T | undefined {
  return i?.workspaceFolderValue ?? i?.workspaceValue ?? i?.globalValue ?? i?.defaultValue;
}

export interface SettingRow {
  key: string;
  value: unknown;
  source: Source;
}

/** Every `bragi.*` setting in the manifest with its effective value and where it came from. */
export function settingRows(properties: Record<string, unknown>, scope?: vscode.ConfigurationScope): SettingRow[] {
  const cfg = vscode.workspace.getConfiguration('bragi', scope);
  return Object.keys(properties)
    .filter((k) => k.startsWith('bragi.'))
    .map((full) => {
      const key = full.slice('bragi.'.length);
      const i = cfg.inspect(key);
      return { key: full, value: effectiveOf(i), source: sourceOf(i) };
    });
}

export function manifestProperties(extension: vscode.Extension<unknown>): Record<string, any> {
  const c = extension.packageJSON?.contributes?.configuration;
  const sections: any[] = Array.isArray(c) ? c : c ? [c] : [];
  return Object.assign({}, ...sections.map((s) => s.properties ?? {}));
}

export interface McpServerOptions {
  allowWrites: boolean;
  allowDeletes: boolean;
  extraArgs: string[];
}

export function mcpServerOptions(): McpServerOptions {
  const cfg = vscode.workspace.getConfiguration('bragi');
  const extra = cfg.get<unknown[]>('mcp.server.args', []);
  return {
    allowWrites: cfg.get<boolean>('mcp.server.allowWrites', false),
    allowDeletes: cfg.get<boolean>('mcp.server.allowDeletes', false),
    extraArgs: Array.isArray(extra) ? extra.filter((a): a is string => typeof a === 'string') : [],
  };
}

/** The arguments after `bragi` for `bragi mcp`, from the settings. */
export function mcpArgs(root: string | undefined, o: McpServerOptions = mcpServerOptions()): string[] {
  const args = ['mcp'];
  if (root) {
    args.push('--root', root);
  }
  if (o.allowWrites) {
    args.push('--allow-writes');
  }
  if (o.allowDeletes) {
    args.push('--allow-deletes');
  }
  return [...args, ...o.extraArgs];
}

export type McpClientKind = 'cursor' | 'vscode' | 'claude';

/** The config a client pastes to run `bragi mcp`. Cursor and Claude Desktop use `mcpServers`, VS Code `servers`. */
export function mcpClientConfig(kind: McpClientKind, program: string, args: string[]): { file: string; json: string } {
  const entry = { command: program, args };
  switch (kind) {
    case 'cursor':
      return { file: '.cursor/mcp.json (or ~/.cursor/mcp.json)', json: JSON.stringify({ mcpServers: { bragi: entry } }, null, 2) };
    case 'vscode':
      return { file: '.vscode/mcp.json', json: JSON.stringify({ servers: { bragi: { type: 'stdio', ...entry } } }, null, 2) };
    case 'claude':
      return { file: 'claude_desktop_config.json', json: JSON.stringify({ mcpServers: { bragi: entry } }, null, 2) };
  }
}

export interface ClientServerSetting {
  name: string;
  transport: 'stdio' | 'http';
  enabled?: boolean;
  command?: string;
  args?: string[];
  cwd?: string;
  url?: string;
  scope?: 'all' | 'folder';
}

/** The `bragi.mcp.clientServers` entries that are well formed; the rest come back as problems. */
export function clientServersFrom(value: unknown): { servers: ClientServerSetting[]; problems: string[] } {
  const servers: ClientServerSetting[] = [];
  const problems: string[] = [];
  if (!Array.isArray(value)) {
    return { servers, problems };
  }
  value.forEach((raw, i) => {
    const s = raw as Partial<ClientServerSetting> | null;
    const name = typeof s?.name === 'string' ? s.name.trim() : '';
    if (!name) {
      problems.push(`entry ${i + 1} has no name`);
    } else if (s?.transport === 'stdio' && !s.command) {
      problems.push(`${name}: a stdio server needs a command`);
    } else if (s?.transport === 'http' && !s.url) {
      problems.push(`${name}: an http server needs a url`);
    } else if (s?.transport !== 'stdio' && s?.transport !== 'http') {
      problems.push(`${name}: transport must be "stdio" or "http"`);
    } else {
      servers.push({ ...(s as ClientServerSetting), name });
    }
  });
  return { servers, problems };
}
