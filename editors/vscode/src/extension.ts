// Bragi for VS Code and Cursor: inline suggestions, rewrites with a diff preview, and a chat
// panel with editor context, answered through the local Bragi app (`bragi serve`) or, if set up,
// an OpenAI-compatible server directly.

import * as vscode from 'vscode';
import { Router } from './backends';
import { ChatView } from './chat';
import { editorContext } from './context';
import { InlineProvider } from './inline';
import { PROPOSED_SCHEME, Rewrites } from './rewrite';
import { defaultTokenPath, Secrets } from './tokens';
import { BragiError, Scope } from './types';

export interface BragiApi {
  router: Router;
  secrets: Secrets;
  inline: InlineProvider;
  rewrites: Rewrites;
  chat: ChatView;
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
    actions.push('How to Start Bragi');
  }
  const pick = await vscode.window.showErrorMessage(`Bragi: ${describe(e)}`, ...actions);
  if (pick === 'Import Token') {
    await vscode.commands.executeCommand('bragi.importToken');
  } else if (pick === 'Set Token') {
    await vscode.commands.executeCommand('bragi.setToken');
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
    const model = router.modelFor(router.target() === 'direct' ? router.direct : router.bragi) ?? 'default model';
    return router.target() === 'direct' ? `Direct · ${model}` : `Bragi · ${model}`;
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
      const up = await router.bragi.health();
      if (!up) {
        status.text = '$(debug-disconnect) Bragi';
        status.tooltip = 'The local Bragi API is not running (start the app with --serve, or run bragi serve)';
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

  context.subscriptions.push(
    status,
    vscode.languages.registerInlineCompletionItemProvider({ pattern: '**' }, inline),
    vscode.workspace.registerTextDocumentContentProvider(PROPOSED_SCHEME, rewrites),
    vscode.window.registerWebviewViewProvider(ChatView.id, chat, { webviewOptions: { retainContextWhenHidden: true } }),
    vscode.commands.registerCommand('bragi.rewriteSelection', rewrite('selection')),
    vscode.commands.registerCommand('bragi.rewriteFile', rewrite('file')),
    vscode.commands.registerCommand('bragi.acceptRewrite', run((uri?: vscode.Uri) => rewrites.accept(uri instanceof vscode.Uri ? uri : undefined))),
    vscode.commands.registerCommand('bragi.rejectRewrite', run((uri?: vscode.Uri) => rewrites.reject(uri instanceof vscode.Uri ? uri : undefined))),
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
    vscode.commands.registerCommand('bragi.pickModel', run(() => pickModel(router))),
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
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration('bragi')) {
        void refreshStatus();
      }
    }),
  );
  const timer = setInterval(() => void refreshStatus(), 30_000);
  context.subscriptions.push({ dispose: () => clearInterval(timer) });
  void refreshStatus();
  return { router, secrets, inline, rewrites, chat, refreshStatus };
}

interface Pick extends vscode.QuickPickItem {
  target?: 'bragi' | 'direct';
  model?: string;
}

async function pickModel(router: Router): Promise<void> {
  const items: Pick[] = [{ label: 'Bragi (local app)', kind: vscode.QuickPickItemKind.Separator }];
  try {
    const list = await router.bragi.models();
    items.push({ label: '$(star) Default model', description: list.defaultModel ?? list.message, target: 'bragi', model: '' });
    for (const m of list.models) {
      items.push({ label: m.id, description: m.status, target: 'bragi', model: m.id });
    }
  } catch (e) {
    items.push({ label: '$(warning) Not available', detail: describe(e), target: 'bragi', model: '' });
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
  const cfg = vscode.workspace.getConfiguration('bragi');
  await cfg.update('target', pick.target, vscode.ConfigurationTarget.Global);
  await cfg.update(pick.target === 'direct' ? 'direct.model' : 'model', pick.model ?? '', vscode.ConfigurationTarget.Global);
}

export function deactivate(): void {}
