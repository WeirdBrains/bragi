// The chat panel: a conversation with editor context, kept in Bragi's history on the Bragi target.

import * as vscode from 'vscode';
import { Backend, Router } from './backends';
import { editorContext } from './context';
import { BragiError, ChatEvent, EditorContext } from './types';

/** What the panel shows; the controller sends these. */
export type ViewMessage =
  | { type: 'user'; text: string; context?: string }
  | { type: 'start'; model?: string; node?: string }
  | { type: 'answer'; text: string }
  | { type: 'status'; text: string }
  | { type: 'done' }
  | { type: 'error'; message: string }
  | { type: 'reset' }
  | { type: 'target'; label: string }
  | { type: 'prefill'; text: string };

/** The chat's logic, without the webview (so it can be tested). */
export class ChatController {
  private conversation?: string;
  private backendId?: Backend['id'];
  private abort?: AbortController;
  private requestId?: string;

  constructor(
    private readonly router: Router,
    private readonly post: (m: ViewMessage) => void,
  ) {}

  get busy(): boolean {
    return this.abort !== undefined;
  }

  get conversationId(): string | undefined {
    return this.conversation;
  }

  reset(): void {
    this.stop();
    this.conversation = undefined;
    this.backendId = undefined;
    this.post({ type: 'reset' });
  }

  stop(): void {
    const id = this.requestId;
    this.abort?.abort();
    this.abort = undefined;
    if (id && this.backendId === 'bragi') {
      void this.router.bragi.cancel(id).catch(() => undefined);
    }
  }

  /** Sends `prompt`, with `context` when given; resolves when the answer is over. Returns its text. */
  async send(prompt: string, context?: EditorContext): Promise<string> {
    prompt = prompt.trim();
    if (!prompt || this.busy) {
      return '';
    }
    const label = context ? (context.selection ? `selection in ${context.path}` : context.path) : undefined;
    this.post({ type: 'user', text: prompt, context: label });
    const abort = new AbortController();
    this.abort = abort;
    let text = '';
    try {
      const backend = await this.router.backend(abort.signal);
      if (this.backendId && this.backendId !== backend.id) {
        // The target changed: a conversation doesn't move between them.
        this.conversation = undefined;
      }
      this.backendId = backend.id;
      const onEvent = (e: ChatEvent) => {
        switch (e.type) {
          case 'conversation':
            this.conversation = (e as { id: string }).id;
            this.requestId = (e as { request_id: string }).request_id;
            break;
          case 'started': {
            const s = e as { model: string; node: string };
            this.post({ type: 'start', model: s.model, node: s.node });
            break;
          }
          case 'queued':
            this.post({ type: 'status', text: `Queued (position ${(e as { position: number }).position})` });
            break;
          case 'retry':
            this.post({ type: 'status', text: `Retrying: ${(e as { message: string }).message}` });
            break;
          case 'delta': {
            const d = e as { offset: number; text: string };
            text = Array.from(text).slice(0, d.offset).join('') + d.text;
            this.post({ type: 'answer', text });
            break;
          }
          case 'error': {
            const er = e as { kind: string; message: string };
            throw new BragiError(er.message, er.kind);
          }
          default:
            break;
        }
      };
      await backend.chat({ prompt, context, conversation: this.conversation, model: this.router.modelFor(backend) }, onEvent, abort.signal);
      this.post({ type: 'done' });
    } catch (e) {
      const err = e instanceof BragiError ? e : new BragiError(e instanceof Error ? e.message : String(e), e instanceof Error && e.name === 'AbortError' ? 'cancelled' : 'other');
      this.post(err.kind === 'cancelled' ? { type: 'done' } : { type: 'error', message: err.message });
    } finally {
      if (this.abort === abort) {
        this.abort = undefined;
      }
      this.requestId = undefined;
    }
    return text;
  }
}

function nonce(): string {
  const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
  let s = '';
  for (let i = 0; i < 32; i++) {
    s += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return s;
}

export class ChatView implements vscode.WebviewViewProvider {
  static readonly id = 'bragi.chat';
  private view?: vscode.WebviewView;
  private queued: ViewMessage[] = [];
  readonly controller: ChatController;

  constructor(
    private readonly extensionUri: vscode.Uri,
    router: Router,
    private readonly targetLabel: () => string,
  ) {
    this.controller = new ChatController(router, (m) => this.post(m));
  }

  private post(m: ViewMessage): void {
    if (this.view) {
      void this.view.webview.postMessage(m);
    } else {
      this.queued.push(m);
    }
  }

  refreshTarget(): void {
    this.post({ type: 'target', label: this.targetLabel() });
  }

  /** Opens the panel with `text` ready in the input. */
  async prefill(text: string): Promise<void> {
    await vscode.commands.executeCommand(`${ChatView.id}.focus`);
    this.post({ type: 'prefill', text });
  }

  resolveWebviewView(view: vscode.WebviewView): void {
    this.view = view;
    const media = vscode.Uri.joinPath(this.extensionUri, 'media');
    view.webview.options = { enableScripts: true, localResourceRoots: [media] };
    const n = nonce();
    const script = view.webview.asWebviewUri(vscode.Uri.joinPath(media, 'chat.js'));
    const style = view.webview.asWebviewUri(vscode.Uri.joinPath(media, 'chat.css'));
    view.webview.html = `<!DOCTYPE html>
<html lang="en"><head><meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${view.webview.cspSource}; script-src 'nonce-${n}';">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<link rel="stylesheet" href="${style}"><title>Bragi</title></head>
<body>
<header><button id="target" title="Choose target and model"></button><button id="new" title="New chat">New</button></header>
<main id="log" aria-live="polite"></main>
<footer>
<textarea id="input" rows="3" placeholder="Ask about your code (Enter sends, Shift+Enter is a new line)"></textarea>
<div class="row"><label><input type="checkbox" id="ctx" checked> Include editor context</label>
<span class="grow"></span><button id="stop" hidden>Stop</button><button id="send">Send</button></div>
</footer>
<script nonce="${n}" src="${script}"></script>
</body></html>`;
    view.webview.onDidReceiveMessage(async (m: { type: string; prompt?: string; context?: boolean; code?: string }) => {
      switch (m.type) {
        case 'ready':
          this.refreshTarget();
          for (const q of this.queued.splice(0)) {
            void view.webview.postMessage(q);
          }
          break;
        case 'send': {
          const editor = vscode.window.activeTextEditor;
          const ctx = m.context && editor ? editorContext(editor) : undefined;
          await this.controller.send(m.prompt ?? '', ctx);
          break;
        }
        case 'stop':
          this.controller.stop();
          break;
        case 'new':
          this.controller.reset();
          break;
        case 'pickModel':
          await vscode.commands.executeCommand('bragi.pickModel');
          break;
        case 'insert': {
          const editor = vscode.window.activeTextEditor ?? vscode.window.visibleTextEditors[0];
          if (editor && m.code !== undefined) {
            await editor.edit((b) => b.replace(editor.selection, m.code!));
          }
          break;
        }
        case 'copy':
          if (m.code !== undefined) {
            await vscode.env.clipboard.writeText(m.code);
          }
          break;
      }
    });
  }
}
