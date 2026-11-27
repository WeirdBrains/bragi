// Commands for setting Bragi up from the editor: links, OpenAI-compatible connections, the local
// server, the MCP server and outside MCP servers, and the settings. Each takes its input as an
// argument too (for tests and other extensions); without one it asks.

import * as vscode from 'vscode';
import { Manage, type EndpointInput, type EndpointMode, type EndpointView, type FolderView, type McpServerInput, type PairView, type SessionSummary } from './manage';
import { clientServersFrom, manifestProperties, mcpArgs, mcpClientConfig, mcpServerOptions, settingRows, type McpClientKind } from './settings';
import type { ServeManager } from './serve';

export interface Deps {
  manage: Manage;
  serve: ServeManager;
  /** The program that runs `bragi` (the `bragi.path` setting, else the PATH). */
  program: () => string;
  /** Redraws the Connections view and the status bar. */
  refresh: () => void;
  /** Wraps a command so its errors are shown the way the rest of the extension shows them. */
  run: (fn: (...args: any[]) => Promise<unknown>) => (...args: any[]) => Promise<unknown>;
  /** Makes sure there is an API token (asks for one in a field when there isn't). False when dismissed. */
  token: () => Promise<boolean>;
  extension: () => vscode.Extension<unknown> | undefined;
}

const MODES: { label: string; value: EndpointMode; detail: string }[] = [
  { label: 'Alongside the pool', value: 'alongside', detail: 'Offered as a target next to your Weirdnet pool; its models are listed.' },
  { label: 'Fallback only', value: 'fallback', detail: "Used only when the Weirdnet pool can't answer." },
];

let mcpTerminal: vscode.Terminal | undefined;

/** Whether this editor has `bragi mcp` running in its terminal. */
export const mcpServerRunning = (): boolean => mcpTerminal !== undefined;

const workspaceRoot = () => vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;


/** Asks for one line; undefined when dismissed. */
async function ask(prompt: string, opts: vscode.InputBoxOptions = {}): Promise<string | undefined> {
  return vscode.window.showInputBox({ prompt, ignoreFocusOut: true, ...opts });
}

export async function askEndpoint(existing?: EndpointView): Promise<EndpointInput | undefined> {
  const name = await ask('Name for this connection', { value: existing?.name, validateInput: (v) => (v.trim() ? undefined : 'A name is needed') });
  if (name === undefined) {
    return undefined;
  }
  const baseUrl = await ask('Base URL of the OpenAI-compatible server, e.g. http://127.0.0.1:8484/v1', {
    value: existing?.base_url,
    validateInput: (v) => (/^https?:\/\/\S+$/.test(v.trim()) ? undefined : 'Use an http:// or https:// address, without credentials'),
  });
  if (baseUrl === undefined) {
    return undefined;
  }
  const model = await ask('Default model (optional)', { value: existing?.model ?? '' });
  if (model === undefined) {
    return undefined;
  }
  const mode = await vscode.window.showQuickPick(MODES, { title: 'When is it used?', ignoreFocusOut: true });
  if (!mode) {
    return undefined;
  }
  const key = await ask(existing?.has_key ? 'API key (empty keeps the stored one)' : 'API key (optional)', { password: true });
  if (key === undefined) {
    return undefined;
  }
  return { name: name.trim(), baseUrl: baseUrl.trim(), model: model.trim() || undefined, mode: mode.value, enabled: existing?.enabled ?? true, apiKey: key.trim() || undefined };
}

/** Each folder's full name, `Work / Bragi`. */
export function folderPaths(folders: FolderView[]): Map<string, string> {
  const byId = new Map(folders.map((f) => [f.id, f]));
  const out = new Map<string, string>();
  for (const f of folders) {
    const names: string[] = [];
    const seen = new Set<string>();
    for (let at: FolderView | undefined = f; at && !seen.has(at.id); at = at.parent ? byId.get(at.parent) : undefined) {
      seen.add(at.id);
      names.unshift(at.name);
    }
    out.set(f.id, names.join(' / '));
  }
  return out;
}

/** Chats grouped under their folders (no folder first), for a quick pick. */
export function sessionItems(sessions: SessionSummary[], folders: FolderView[]): (vscode.QuickPickItem & { s?: SessionSummary })[] {
  const paths = folderPaths(folders);
  const groups = new Map<string, SessionSummary[]>();
  for (const s of sessions) {
    const key = s.folder && paths.has(s.folder) ? paths.get(s.folder)! : '';
    groups.set(key, [...(groups.get(key) ?? []), s]);
  }
  const items: (vscode.QuickPickItem & { s?: SessionSummary })[] = [];
  for (const key of [...groups.keys()].sort()) {
    items.push({ label: key || 'No folder', kind: vscode.QuickPickItemKind.Separator });
    for (const s of groups.get(key)!) {
      items.push({ label: s.title || '(untitled)', description: s.target_label, detail: `${s.messages} messages`, s });
    }
  }
  return items;
}

async function pickFolder(manage: Manage, title: string, none?: string): Promise<{ id: string | null } | undefined> {
  const folders = await manage.folders();
  const paths = folderPaths(folders);
  const items: (vscode.QuickPickItem & { id: string | null })[] = [
    ...(none ? [{ label: none, id: null }] : []),
    ...folders.map((f) => ({ label: paths.get(f.id) ?? f.name, id: f.id })).sort((a, b) => a.label.localeCompare(b.label)),
  ];
  if (items.length === 0) {
    void vscode.window.showInformationMessage('Bragi has no chat folders yet. Run "Bragi: New Chat Folder…".');
    return undefined;
  }
  return vscode.window.showQuickPick(items, { title });
}

async function pickEndpoint(manage: Manage, id?: string): Promise<EndpointView | undefined> {
  const all = await manage.endpoints();
  if (id) {
    return all.find((e) => e.id === id);
  }
  if (all.length === 0) {
    void vscode.window.showInformationMessage('Bragi has no OpenAI connections yet. Run "Bragi: Add OpenAI Connection".');
    return undefined;
  }
  const pick = await vscode.window.showQuickPick(
    all.map((e) => ({ label: e.name, description: e.base_url, detail: `${e.mode === 'alongside' ? 'Alongside the pool' : 'Fallback only'}${e.has_key ? ' · key set' : ''}${e.enabled ? '' : ' · off'}`, e })),
    { title: 'Choose a connection' },
  );
  return pick?.e;
}

export function register(context: vscode.ExtensionContext, d: Deps): void {
  const { manage, serve } = d;
  // Commands that talk to `bragi serve` make sure there is a token first; `local` ones don't.
  const cmd = (id: string, fn: (...args: any[]) => Promise<unknown>, local = false) =>
    context.subscriptions.push(
      vscode.commands.registerCommand(
        id,
        d.run(async (...args: any[]) => {
          if (!local && !(await d.token())) {
            return undefined;
          }
          return fn(...args);
        }),
      ),
    );
  const done = (message: string) => {
    d.refresh();
    void vscode.window.showInformationMessage(`Bragi: ${message}`);
  };

  // ---- links ----
  cmd('bragi.link.add', async (code?: string, matches?: boolean) => {
    const text = code ?? (await ask('Paste the link code from your other device (or a Ymir node: `ymirctl pool addresses`)'));
    if (!text) {
      return;
    }
    const { sas, device } = await manage.linkAccept(text);
    const same =
      matches ??
      (await vscode.window.showInformationMessage(`Does ${device ? `"${device}"` : 'the other device'} show this code: ${sas}?`, { modal: true }, 'Yes, they match')) === 'Yes, they match';
    await manage.linkConfirm(same);
    done(same ? 'linked.' : "the codes didn't match, so nothing was linked.");
  });
  // Show a code for a device already in your Weirdnet; it reads the code, both screens show a
  // short code to compare, and confirming here links this Bragi.
  cmd('bragi.link.offer', async (matches?: boolean) => {
    const { link, expires_ms } = await manage.linkOffer();
    await vscode.env.clipboard.writeText(link);
    const until = expires_ms ? ` It works until ${new Date(expires_ms).toLocaleTimeString()}.` : '';
    const pair = await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Notification, title: `Bragi link code (copied): ${link}`, cancellable: true },
      async (progress, cancel) => {
        progress.report({ message: `Enter it on the device that is already linked (Morph, Ymir or another Bragi).${until}` });
        for (;;) {
          if (cancel.isCancellationRequested) {
            return (await manage.linkOfferCancel()).pair;
          }
          const p = await manage.pair();
          if (p.state !== 'offering') {
            return p;
          }
          await new Promise((r) => setTimeout(r, 400));
        }
      },
    );
    let now: PairView = pair;
    if (now.state === 'confirm') {
      const sas = now.sas;
      const same = matches ?? (await vscode.window.showInformationMessage(`Does the other device show this code: ${sas}?`, { modal: true }, 'Yes, they match')) === 'Yes, they match';
      now = (await manage.linkOfferConfirm(same)).pair;
      for (let i = 0; i < 150 && now.state === 'waiting'; i++) {
        await new Promise((r) => setTimeout(r, 200));
        now = await manage.pair();
      }
    }
    switch (now.state) {
      case 'linked':
        done(`linked${now.device ? ` as "${now.device}"` : ''}.`);
        break;
      case 'failed':
        void vscode.window.showWarningMessage(`Bragi: ${now.message}`);
        break;
      case 'waiting':
        void vscode.window.showInformationMessage('Bragi: confirmed here; finish on the other device.');
        break;
      default:
        d.refresh();
    }
    return now;
  });
  cmd('bragi.link.addNode', async (address?: string) => {
    const a = address ?? (await ask('Address of an inference node (`ymirctl pool addresses` prints it)'));
    if (!a) {
      return;
    }
    done((await manage.addNode(a)).message);
  });
  cmd('bragi.link.remove', async (address?: string | { key?: string }) => {
    let target = typeof address === 'string' ? address : address?.key;
    if (!target) {
      const links = await manage.links();
      const pick = await vscode.window.showQuickPick(
        links.map((l) => ({ label: l.address, description: l.online ? 'online' : 'offline', l })),
        { title: 'Remove which link?' },
      );
      target = pick?.l.address;
    }
    if (!target) {
      return;
    }
    await manage.removeLink(target);
    done('link removed.');
  });

  // ---- OpenAI connections ----
  cmd('bragi.endpoint.add', async (input?: EndpointInput) => {
    const i = input ?? (await askEndpoint());
    if (!i) {
      return;
    }
    const v = await manage.addEndpoint(i);
    done(`added ${v.name}${v.insecure ? ' (plain http to another machine: the key and chat can be read on the way)' : ''}.`);
  });
  cmd('bragi.endpoint.edit', async (idOrItem?: string | { key?: string }, input?: EndpointInput) => {
    const e = await pickEndpoint(manage, typeof idOrItem === 'string' ? idOrItem : idOrItem?.key);
    if (!e) {
      return;
    }
    const i = input ?? (await askEndpoint(e));
    if (!i) {
      return;
    }
    await manage.editEndpoint(e.id, i);
    done(`saved ${i.name}.`);
  });
  cmd('bragi.endpoint.remove', async (idOrItem?: string | { key?: string }, confirmed?: boolean) => {
    const e = await pickEndpoint(manage, typeof idOrItem === 'string' ? idOrItem : idOrItem?.key);
    if (!e) {
      return;
    }
    const ok = confirmed ?? (await vscode.window.showWarningMessage(`Remove "${e.name}" and its key? Chats bound to it will ask for a new target.`, { modal: true }, 'Remove')) === 'Remove';
    if (!ok) {
      return;
    }
    await manage.removeEndpoint(e.id);
    done(`removed ${e.name}.`);
  });
  cmd('bragi.endpoint.test', async (idOrItem?: string | { key?: string }) => {
    const e = await pickEndpoint(manage, typeof idOrItem === 'string' ? idOrItem : idOrItem?.key);
    if (!e) {
      return;
    }
    const r = await manage.testEndpoint(e.id);
    void (r.ok
      ? vscode.window.showInformationMessage(`Bragi: ${e.name} answered in ${r.latency_ms} ms with ${r.models.length} model(s).`)
      : vscode.window.showWarningMessage(`Bragi: ${e.name}: ${r.message}`));
    return r;
  });

  // ---- chats ----
  cmd('bragi.sessions.manage', async () => {
    const list = await manage.sessions();
    const pick = await vscode.window.showQuickPick(
      list.map((s) => ({ label: s.title || '(untitled)', description: s.target_label, detail: `${s.messages} messages`, s })),
      { title: 'Chats in Bragi: pick one to delete' },
    );
    if (!pick) {
      return;
    }
    if ((await vscode.window.showWarningMessage(`Delete "${pick.s.title}"? It can be undone for a short time in the Bragi app.`, { modal: true }, 'Delete')) === 'Delete') {
      await manage.deleteSession(pick.s.id);
      done('chat deleted.');
    }
  });

  // ---- chat folders ----
  cmd('bragi.folders.new', async (name?: string, parent?: string | null) => {
    const n = name ?? (await ask('Name for the new chat folder', { validateInput: (v) => (v.trim() ? undefined : 'A name is needed') }));
    if (!n) {
      return undefined;
    }
    let under = parent;
    if (under === undefined && name === undefined && (await manage.folders()).length) {
      const pick = await pickFolder(manage, 'Put it in…', 'Top level');
      if (!pick) {
        return undefined;
      }
      under = pick.id;
    }
    const f = await manage.createFolder(n.trim(), under ?? undefined);
    done(`folder "${f.name}" made.`);
    return f;
  });
  cmd('bragi.folders.rename', async (id?: string, name?: string) => {
    const f = id ? { id } : await pickFolder(manage, 'Rename which folder?');
    if (!f?.id) {
      return;
    }
    const n = name ?? (await ask('New name', { validateInput: (v) => (v.trim() ? undefined : 'A name is needed') }));
    if (!n) {
      return;
    }
    await manage.renameFolder(f.id, n.trim());
    done('folder renamed.');
  });
  cmd('bragi.folders.move', async (id?: string, parent?: string | null) => {
    const f = id ? { id } : await pickFolder(manage, 'Move which folder?');
    if (!f?.id) {
      return;
    }
    const to = parent !== undefined ? { id: parent } : await pickFolder(manage, 'Into…', 'Top level');
    if (!to) {
      return;
    }
    await manage.moveFolder(f.id, to.id);
    done('folder moved.');
  });
  cmd('bragi.folders.delete', async (id?: string) => {
    const f = id ? { id } : await pickFolder(manage, 'Delete which folder? (its chats move up, none are deleted)');
    if (!f?.id) {
      return;
    }
    if (!id && (await vscode.window.showWarningMessage('Delete this folder? Its chats and folders move up one level.', { modal: true }, 'Delete')) !== 'Delete') {
      return;
    }
    await manage.deleteFolder(f.id);
    done('folder deleted.');
  });
  cmd('bragi.sessions.moveToFolder', async (session?: string, folder?: string | null) => {
    let id = session;
    if (!id) {
      const pick = await vscode.window.showQuickPick(sessionItems(await manage.sessions(), await manage.folders()), { title: 'Move which chat?' });
      id = pick?.s?.id;
    }
    if (!id) {
      return;
    }
    const to = folder !== undefined ? { id: folder } : await pickFolder(manage, 'Move it to…', 'No folder');
    if (!to) {
      return;
    }
    await manage.setSessionFolder(id, to.id);
    done(to.id ? 'chat moved.' : 'chat taken out of its folder.');
  });

  // ---- the local server ----
  cmd('bragi.serve.stop', async () => {
    const s = await serve.stop();
    d.refresh();
    return s;
  }, true);
  cmd('bragi.serve.status', async () => {
    const s = await serve.check();
    const text = s.kind === 'running' ? `running on port ${serve.port}${s.started ? ' (started by this editor)' : ''}` : `${s.kind}${s.message ? `: ${s.message}` : ''}`;
    void vscode.window.showInformationMessage(`Bragi's server is ${text}.`);
    return s;
  }, true);

  // ---- Bragi as an MCP server ----
  cmd('bragi.mcp.start', async () => {
    mcpTerminal?.dispose();
    const args = mcpArgs(workspaceRoot());
    mcpTerminal = vscode.window.createTerminal({ name: 'Bragi MCP server', shellPath: d.program(), shellArgs: args });
    mcpTerminal.show(true);
    d.refresh();
    return args;
  }, true);
  cmd('bragi.mcp.stop', async () => {
    mcpTerminal?.dispose();
    mcpTerminal = undefined;
    d.refresh();
  }, true);
  context.subscriptions.push(
    vscode.window.onDidCloseTerminal((t) => {
      if (t === mcpTerminal) {
        mcpTerminal = undefined;
        d.refresh();
      }
    }),
  );
  cmd('bragi.mcp.copyConfig', async (kind?: McpClientKind) => {
    const k =
      kind ??
      (
        await vscode.window.showQuickPick(
          [
            { label: 'Cursor', k: 'cursor' as const, description: '.cursor/mcp.json' },
            { label: 'VS Code', k: 'vscode' as const, description: '.vscode/mcp.json' },
            { label: 'Claude Desktop', k: 'claude' as const, description: 'claude_desktop_config.json' },
          ],
          { title: 'Copy the MCP config for…' },
        )
      )?.k;
    if (!k) {
      return undefined;
    }
    const root = k === 'cursor' ? '${workspaceFolder}' : k === 'vscode' ? '${workspaceFolder}' : workspaceRoot();
    const { file, json } = mcpClientConfig(k, d.program(), mcpArgs(root, mcpServerOptions()));
    await vscode.env.clipboard.writeText(json);
    void vscode.window.showInformationMessage(`Bragi: copied the config. Paste it into ${file}.`);
    return json;
  }, true);
  cmd('bragi.mcp.isRunning', async () => mcpTerminal !== undefined, true);

  // ---- outside MCP servers ----
  cmd('bragi.mcpServer.add', async (input?: McpServerInput) => {
    let i = input;
    if (!i) {
      const transport = await vscode.window.showQuickPick([{ label: 'A local command (stdio)', t: 'stdio' as const }, { label: 'A web address (streamable HTTP)', t: 'http' as const }], { title: 'Add an MCP server' });
      if (!transport) {
        return;
      }
      const name = await ask('Name', { validateInput: (v) => (v.trim() ? undefined : 'A name is needed') });
      if (!name) {
        return;
      }
      if (transport.t === 'stdio') {
        const line = await ask('Command and arguments, e.g. npx -y @modelcontextprotocol/server-filesystem /path');
        if (!line?.trim()) {
          return;
        }
        const [command, ...args] = line.trim().split(/\s+/);
        const ok = await vscode.window.showWarningMessage(`"${name}" will run \`${line.trim()}\` as you, with your permissions. Add it?`, { modal: true }, 'Add');
        if (ok !== 'Add') {
          return;
        }
        i = { name: name.trim(), transport: 'stdio', command, args };
      } else {
        const url = await ask('Server address, e.g. https://example.com/mcp');
        if (!url) {
          return;
        }
        const bearer = await ask('Bearer token (optional; kept in Bragi\'s secret store)', { password: true });
        i = { name: name.trim(), transport: 'http', url: url.trim(), secrets: bearer?.trim() ? { bearer: bearer.trim() } : undefined };
      }
      const root = workspaceRoot();
      if (root) {
        const where = await vscode.window.showQuickPick(
          [
            { label: 'Every chat', s: 'all' as const },
            { label: 'Only code chats in this folder', description: root, s: 'folder' as const },
          ],
          { title: `Where may the model use ${i.name}'s tools?` },
        );
        if (!where) {
          return;
        }
        i = { ...i, scope: where.s, folder: where.s === 'folder' ? root : undefined };
      }
    }
    const v = await manage.addMcpServer(i);
    done(`added MCP server ${v.name}. Tool calls ask before they run.`);
  });
  cmd('bragi.mcpServer.remove', async (idOrItem?: string | { key?: string }) => {
    const id = typeof idOrItem === 'string' ? idOrItem : idOrItem?.key;
    const all = await manage.mcpServers();
    const s = id ? all.find((x) => x.id === id) : (await vscode.window.showQuickPick(all.map((x) => ({ label: x.name, description: x.transport, x })), { title: 'Remove which MCP server?' }))?.x;
    if (!s) {
      return;
    }
    await manage.removeMcpServer(s.id);
    done(`removed ${s.name}.`);
  });
  cmd('bragi.mcpServer.toggle', async (idOrItem?: string | { key?: string }) => {
    const id = typeof idOrItem === 'string' ? idOrItem : idOrItem?.key;
    const all = await manage.mcpServers();
    const s = id ? all.find((x) => x.id === id) : (await vscode.window.showQuickPick(all.map((x) => ({ label: x.name, description: x.enabled ? 'on' : 'off', x })), { title: 'Turn which MCP server on or off?' }))?.x;
    if (!s) {
      return;
    }
    await manage.setMcpServerEnabled(s.id, !s.enabled);
    done(`${s.name} is ${s.enabled ? 'off' : 'on'}.`);
  });
  cmd('bragi.mcpServer.test', async (idOrItem?: string | { key?: string }) => {
    const id = typeof idOrItem === 'string' ? idOrItem : idOrItem?.key;
    if (!id) {
      return;
    }
    const r = await manage.testMcpServer(id);
    void vscode.window.showInformationMessage(`Bragi: ${r.ok ? `${r.tools.length} tool(s): ${r.tools.slice(0, 8).join(', ')}` : r.message}`);
    return r;
  });
  cmd('bragi.mcpServer.alwaysAllow', async (idOrItem?: string | { key?: string }, tool?: string) => {
    const id = typeof idOrItem === 'string' ? idOrItem : idOrItem?.key;
    const all = await manage.mcpServers();
    const s = id ? all.find((x) => x.id === id) : (await vscode.window.showQuickPick(all.filter((x) => x.always_allow?.length).map((x) => ({ label: x.name, description: x.always_allow.join(', '), x })), { title: 'Which server\'s "always allow"?' }))?.x;
    if (!s) {
      return;
    }
    const t = tool ?? (await vscode.window.showQuickPick(s.always_allow ?? [], { title: `Ask again before running…`, placeHolder: s.always_allow?.length ? undefined : 'Nothing is always allowed' }));
    if (!t) {
      return;
    }
    await manage.setMcpToolAlwaysAllowed(s.id, t, false);
    done(`${t} asks before it runs again.`);
  });
  cmd('bragi.mcp.applySettings', async () => {
    const { servers, problems } = clientServersFrom(vscode.workspace.getConfiguration('bragi').get('mcp.clientServers', []));
    if (problems.length) {
      void vscode.window.showWarningMessage(`Bragi: skipped ${problems.length} MCP server setting(s): ${problems.join('; ')}`);
    }
    const have = new Set((await manage.mcpServers()).map((s) => s.name));
    let added = 0;
    for (const s of servers) {
      if (!have.has(s.name)) {
        const scope = s.scope ?? 'all';
        await manage.addMcpServer({ ...s, scope, folder: scope === 'folder' ? workspaceRoot() : undefined });
        added++;
      }
    }
    done(added ? `added ${added} MCP server(s) from settings.` : 'Bragi already has every MCP server in settings.');
    return added;
  });

  // ---- settings ----
  cmd('bragi.settings.open', () => Promise.resolve(vscode.commands.executeCommand('workbench.action.openSettings', '@ext:weirdbrains.bragi')), true);
  cmd('bragi.settings.openJson', async () => {
    const choice = await vscode.window.showQuickPick(
      [
        { label: 'User settings.json', description: 'applies everywhere', id: 'workbench.action.openSettingsJson' },
        ...(vscode.workspace.workspaceFolders ? [{ label: 'Workspace settings.json', description: 'this project only (not for machine-scoped keys)', id: 'workbench.action.openWorkspaceSettingsFile' }] : []),
      ],
      { title: 'Open Bragi settings as JSON' },
    );
    if (choice) {
      await vscode.commands.executeCommand(choice.id);
    }
  }, true);
  cmd('bragi.settings.show', async () => {
    const ext = d.extension();
    const rows = ext ? settingRows(manifestProperties(ext), vscode.window.activeTextEditor?.document.uri) : [];
    const pick = await vscode.window.showQuickPick(
      rows.map((r) => ({ label: r.key, description: r.source, detail: JSON.stringify(r.value) ?? '', r })),
      { title: 'Bragi settings: workspace folder > workspace > user > Bragi default', matchOnDetail: true },
    );
    if (pick) {
      await vscode.commands.executeCommand('workbench.action.openSettings', pick.r.key);
    }
    return rows;
  }, true);
}

