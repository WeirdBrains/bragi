// Bragi for VS Code and Cursor: inline suggestions, rewrites reviewed hunk by hunk, and a chat
// panel with editor context, answered through the local Bragi app (`bragi serve`) or, if set up,
// an OpenAI-compatible server directly.

import * as vscode from 'vscode';
import { Router } from './backends';
import { ChatView } from './chat';
import { mcpServerRunning, register as registerManagement } from './commands';
import { Manage } from './manage';
import { ConnectionsView } from './tree';
import { editorContext } from './context';
import { InlineProvider } from './inline';
import { PROPOSED_SCHEME, Rewrites } from './rewrite';
import { ServeManager, describeServe } from './serve';
import { defaultTokenPath, Secrets } from './tokens';
import { BragiError, BragiTarget, Scope } from './types';

export interface BragiApi {
  router: Router;
  secrets: Secrets;
  inline: InlineProvider;
  rewrites: Rewrites;
  chat: ChatView;
  serve: ServeManager;
  refreshStatus(): Promise<void>;
}

function describe(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

async function showError(e: unknown): Promise<void> {
  if (e instanceof BragiError && e.kind === 'cancelled') {
    return;
  }
  const actions: string[] = [];
  if (e instanceof BragiError && e.kind === 'unauthorized') {
    actions.push('Import Token', 'Set Token');
  } else if (e instanceof BragiError && e.kind === 'unreachable') {
    actions.push('Start Server', 'How to Start Bragi');
  }
  const pick = await vscode.window.showErrorMessage(`Bragi: ${describe(e)}`, ...actions);
  if (pick === 'Import Token') {
    await vscode.commands.executeCommand('bragi.importToken');
  } else if (pick === 'Set Token') {
    await vscode.commands.executeCommand('bragi.setToken');
  } else if (pick === 'Start Server') {
    await vscode.commands.executeCommand('bragi.restartServer');
  } else if (pick === 'How to Start Bragi') {
    void vscode.window.showInformationMessage('Start the Bragi app with --serve, or run `bragi serve` (or `bragi-serve`) in a terminal. It listens on 127.0.0.1:4877.', { modal: true });
  }
}

export function activate(context: vscode.ExtensionContext): BragiApi {
  const secrets = new Secrets(context.secrets);
  const router = new Router(secrets);
  const inline = new InlineProvider(router);
  const rewrites = new Rewrites(router);
  const cfg = () => vscode.workspace.getConfiguration('bragi');

  const status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
  status.command = 'bragi.pickModel';
  status.name = 'Bragi';
  const targetLabel = () => {
    if (router.target() === 'direct') {
      return `Direct · ${router.modelFor(router.direct) ?? 'default model'}`;
    }
    const t = cfg().get<BragiTarget | null>('bragiTarget', null);
    if (t?.kind === 'endpoint') {
      return `Bragi · ${t.model || t.id}`;
    }
    return `Bragi · ${router.modelFor(router.bragi) ?? 'default model'}`;
  };
  const serve = new ServeManager(secrets, context.globalState, () => router.target() === 'bragi');
  router.bragi.revive = async () => (await serve.ensure()).kind === 'running';
  const serveItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 99);
  serveItem.name = "Bragi's server";
  serveItem.command = 'bragi.restartServer';
  const drawServe = () => {
    const d = describeServe(serve.state, serve.port);
    serveItem.text = d.text;
    serveItem.tooltip = d.tooltip;
    serveItem.backgroundColor = d.warn ? new vscode.ThemeColor('statusBarItem.warningBackground') : undefined;
    if (router.target() === 'bragi') {
      serveItem.show();
    } else {
      serveItem.hide();
    }
  };
  const chat = new ChatView(context.extensionUri, router, targetLabel);

  async function refreshStatus(): Promise<void> {
    const inlineOn = cfg().get('inline.enabled', true);
    status.text = `${inlineOn ? '$(sparkle)' : '$(circle-slash)'} ${targetLabel()}`;
    status.tooltip = 'Bragi: choose the target and model';
    status.backgroundColor = undefined;
    status.show();
    chat.refreshTarget();
    if (router.target() === 'bragi') {
      // Starts the server when none answers (the setting `bragi.autoStartServer`), else just looks.
      // Later it only looks: a server that stopped itself when idle starts again on the next request.
      const s = serve.state.kind === 'unknown' ? await serve.ensure() : serve.state.kind === 'starting' ? serve.state : await serve.check();
      drawServe();
      const up = s.kind === 'running';
      if (!up) {
        status.text = '$(debug-disconnect) Bragi';
        status.tooltip = s.message ?? 'The local Bragi API is not running (start the app with --serve, or run bragi serve)';
        status.backgroundColor = new vscode.ThemeColor('statusBarItem.warningBackground');
      }
    }
  }

  inline.onError.event((e) => {
    status.tooltip = `Bragi: ${e.message}`;
    status.backgroundColor = new vscode.ThemeColor('statusBarItem.warningBackground');
    if (e.kind === 'unauthorized') {
      void showError(e);
    }
  });

  const run = (fn: (...args: any[]) => Promise<unknown>) =>
    async (...args: any[]) => {
      try {
        return await fn(...args);
      } catch (e) {
        await showError(e);
        return undefined;
      }
    };

  const rewrite = (scope: Scope) =>
    run(async (instruction?: string) => {
      const editor = vscode.window.activeTextEditor;
      if (!editor || editor.document.uri.scheme === PROPOSED_SCHEME) {
        void vscode.window.showWarningMessage('Open a file to rewrite first.');
        return undefined;
      }
      return rewrites.start(editor, scope, typeof instruction === 'string' ? instruction : undefined);
    });

  const manage = new Manage(() => router.bragi);
  const connections = new ConnectionsView(manage, serve, mcpServerRunning);
  registerManagement(context, {
    manage,
    serve,
    program: () => cfg().get<string>('path', '') || 'bragi',
    refresh: () => {
      connections.refresh();
      void refreshStatus();
    },
    run,
    token: () => ensureToken(secrets, serve),
    extension: () => vscode.extensions.getExtension('weirdbrains.bragi'),
  });

  context.subscriptions.push(
    status,
    serveItem,
    serve,
    serve.onDidChange(drawServe),
    vscode.commands.registerCommand('bragi.restartServer', async () => {
      await serve.restart();
      await refreshStatus();
    }),
    vscode.languages.registerInlineCompletionItemProvider({ pattern: '**' }, inline),
    rewrites.register(),
    vscode.window.registerWebviewViewProvider(ChatView.id, chat, { webviewOptions: { retainContextWhenHidden: true } }),
    vscode.commands.registerCommand('bragi.rewriteSelection', rewrite('selection')),
    vscode.commands.registerCommand('bragi.rewriteFile', rewrite('file')),
    vscode.commands.registerCommand('bragi.acceptRewrite', run((uri?: vscode.Uri) => rewrites.accept(uri instanceof vscode.Uri ? uri : undefined))),
    vscode.commands.registerCommand('bragi.rejectRewrite', run((uri?: vscode.Uri) => rewrites.reject(uri instanceof vscode.Uri ? uri : undefined))),
    vscode.commands.registerCommand(
      'bragi.acceptHunk',
      run((uri?: vscode.Uri, index?: number, generation?: number) => rewrites.acceptHunk(uri instanceof vscode.Uri ? uri : undefined, typeof index === 'number' ? index : undefined, generation)),
    ),
    vscode.commands.registerCommand(
      'bragi.rejectHunk',
      run((uri?: vscode.Uri, index?: number, generation?: number) => rewrites.rejectHunk(uri instanceof vscode.Uri ? uri : undefined, typeof index === 'number' ? index : undefined, generation)),
    ),
    vscode.commands.registerCommand('bragi.showRewriteDiff', run((uri?: vscode.Uri) => rewrites.showDiff(uri instanceof vscode.Uri ? uri : undefined))),
    vscode.commands.registerCommand('bragi.openChat', () => vscode.commands.executeCommand(`${ChatView.id}.focus`)),
    vscode.commands.registerCommand(
      'bragi.askAboutSelection',
      run(async (question?: string) => {
        const editor = vscode.window.activeTextEditor;
        if (typeof question === 'string' && question.trim()) {
          return chat.controller.send(question, editor ? editorContext(editor) : undefined);
        }
        await chat.prefill('Explain the selected code.');
        return undefined;
      }),
    ),
    vscode.commands.registerCommand('bragi.toggleInline', async () => {
      await cfg().update('inline.enabled', !cfg().get('inline.enabled', true), vscode.ConfigurationTarget.Global);
    }),
    vscode.commands.registerCommand(
      'bragi.setToken',
      run(async (token?: string) => {
        const t =
          typeof token === 'string'
            ? token
            : await vscode.window.showInputBox({ title: 'Bragi API token', prompt: 'Paste the output of `bragi serve token`', password: true, ignoreFocusOut: true });
        if (t?.trim()) {
          await secrets.setServeToken(t);
          await refreshStatus();
        }
      }),
    ),
    vscode.commands.registerCommand(
      'bragi.importToken',
      run(async (file?: string) => {
        const path = typeof file === 'string' ? file : defaultTokenPath();
        if (await secrets.importServeToken(path)) {
          void vscode.window.showInformationMessage('Bragi: token imported from the Bragi app.');
          await refreshStatus();
          return true;
        }
        void vscode.window.showWarningMessage(`Bragi: no token at ${path}. Start \`bragi serve\` (or the app with --serve) once, or paste one with "Bragi: Set Bragi API Token".`);
        return false;
      }),
    ),
    vscode.commands.registerCommand('bragi.clearToken', async () => {
      await secrets.clearServeToken();
    }),
    vscode.commands.registerCommand(
      'bragi.setDirectKey',
      run(async (key?: string) => {
        const k = typeof key === 'string' ? key : await vscode.window.showInputBox({ title: 'Direct endpoint API key', prompt: 'Leave empty for none', password: true, ignoreFocusOut: true });
        if (k !== undefined) {
          await secrets.setDirectKey(k);
        }
      }),
    ),
    vscode.commands.registerCommand('bragi.pickModel', run(() => pickModel(router, secrets, serve))),
    vscode.commands.registerCommand(
      'bragi.showStatus',
      run(async () => {
        const backend = await router.backend();
        const list = await backend.models();
        const msg = list.models.length
          ? `${backend.label}: ${list.models.length} model(s), default ${list.defaultModel ?? 'none'}.`
          : `${backend.label}: no models. ${list.message ?? ''}`;
        void vscode.window.showInformationMessage(msg);
        return list;
      }),
    ),
    vscode.window.registerTreeDataProvider(ConnectionsView.id, connections),
    vscode.commands.registerCommand('bragi.connections.refresh', () => connections.refresh()),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration('bragi')) {
        connections.refresh();
        void refreshStatus();
      }
    }),
  );
  const timer = setInterval(() => void refreshStatus(), 30_000);
  context.subscriptions.push({ dispose: () => clearInterval(timer) });
  void refreshStatus();
  drawServe();
  return { router, secrets, inline, rewrites, chat, serve, refreshStatus };
}

interface Pick extends vscode.QuickPickItem {
  target?: 'bragi' | 'direct';
  model?: string;
  bragiTarget?: BragiTarget | null;
}

/** How the token is asked for: the typed token, 'import' for the Bragi app's file, or undefined if dismissed. */
export type AskToken = () => Promise<string | { import: true } | undefined>;

const askTokenInBox: AskToken = () =>
  new Promise((resolve) => {
    const box = vscode.window.createInputBox();
    const importBtn: vscode.QuickInputButton = { iconPath: new vscode.ThemeIcon('cloud-download'), tooltip: 'Import the token from the Bragi app' };
    box.title = 'Bragi: no API token yet';
    box.prompt = "There's no Bragi API token yet. Paste it here (`bragi serve token` prints it), or use the button to import it from the Bragi app.";
    box.placeholder = 'Bragi API token';
    box.password = true;
    box.ignoreFocusOut = true;
    box.buttons = [importBtn];
    let done = false;
    const end = (v: string | { import: true } | undefined) => {
      if (!done) {
        done = true;
        resolve(v);
        box.dispose();
      }
    };
    box.onDidAccept(() => (box.value.trim() ? end(box.value.trim()) : end({ import: true })));
    box.onDidTriggerButton(() => end({ import: true }));
    box.onDidHide(() => end(undefined));
    box.show();
  });

/**
 * Makes sure there is a token: the stored one, else the one in the Bragi home (read silently, after
 * the server had its chance to make it), else a prompt. True when there is one. Never shows it.
 */
export async function ensureToken(secrets: Secrets, serve?: ServeManager, ask: AskToken = askTokenInBox): Promise<boolean> {
  if (await secrets.serveToken()) {
    return true;
  }
  if (serve) {
    await serve.ensure();
  }
  if (await secrets.importServeToken()) {
    return true;
  }
  const answer = await ask();
  if (!answer) {
    return false;
  }
  if (typeof answer === 'string') {
    await secrets.setServeToken(answer);
    return true;
  }
  if (await secrets.importServeToken()) {
    return true;
  }
  void vscode.window.showWarningMessage(`Bragi: no token found at ${defaultTokenPath()}. Start Bragi once, or paste the token from \`bragi serve token\`.`);
  return false;
}

async function pickModel(router: Router, secrets: Secrets, serve: ServeManager): Promise<void> {
  if (router.target() === 'bragi' && !(await ensureToken(secrets, serve))) {
    return;
  }
  const items: Pick[] = [{ label: 'Bragi (local app)', kind: vscode.QuickPickItemKind.Separator }];
  try {
    const [list, targets] = await Promise.all([router.bragi.models(), router.bragi.targets()]);
    items.push({ label: '$(star) Default target and model', description: list.defaultModel ?? list.message, target: 'bragi', model: '', bragiTarget: null });
    for (const t of targets) {
      if (t.target.kind === 'pool') {
        for (const m of list.models) {
          items.push({ label: m.id, description: `${t.label} · ${m.status ?? ''}`, target: 'bragi', model: m.id, bragiTarget: t.target });
        }
      } else {
        items.push({ label: t.label, description: 'endpoint from Bragi Settings', target: 'bragi', model: '', bragiTarget: t.target });
      }
    }
  } catch (e) {
    items.push({ label: '$(warning) Not available', detail: describe(e), target: 'bragi', model: '', bragiTarget: null });
  }
  const direct = vscode.workspace.getConfiguration('bragi').get<string>('direct.url', '');
  if (direct) {
    items.push({ label: `Direct: ${direct}`, kind: vscode.QuickPickItemKind.Separator });
    try {
      const list = await router.direct.models();
      for (const m of list.models) {
        items.push({ label: m.id, description: 'direct', target: 'direct', model: m.id });
      }
    } catch (e) {
      items.push({ label: '$(warning) Not available', detail: describe(e), target: 'direct', model: '' });
    }
  }
  const pick = await vscode.window.showQuickPick(items, { title: 'Bragi: target and model', matchOnDescription: true });
  if (!pick?.target) {
    return;
  }
  await applyPick(pick);
}

/** Saves a picked target and model to the settings. */
export async function applyPick(pick: { target?: 'bragi' | 'direct'; model?: string; bragiTarget?: BragiTarget | null }): Promise<void> {
  const cfg = vscode.workspace.getConfiguration('bragi');
  await cfg.update('target', pick.target, vscode.ConfigurationTarget.Global);
  await cfg.update(pick.target === 'direct' ? 'direct.model' : 'model', pick.model ?? '', vscode.ConfigurationTarget.Global);
  if (pick.target === 'bragi') {
    await cfg.update('bragiTarget', pick.bragiTarget ?? null, vscode.ConfigurationTarget.Global);
  }
}

export function deactivate(): void {}
