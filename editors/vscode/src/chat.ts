// The chat panel: a conversation with editor context, kept in Bragi's history on the Bragi target.

import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import { ApprovalChoice, Backend, Router } from './backends';
import { editorContext } from './context';
import { BragiError, ChatEvent, EditorContext, VideoLimits } from './types';
import { droppedClip, isVideoPath, PlayerMessage, playerMessage, resourceRoots, VIDEO_EXTENSIONS, VideoEvent, withSource } from './video';

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
  | { type: 'prefill'; text: string }
  /** A clip waits to go with the next message; `chip` is the model's video limits line. */
  | { type: 'attached'; name: string; chip?: string; ready?: boolean }
  | { type: 'detached' }
  /** Agent mode: the model asks to run a tool; `always` offers "always allow" (an outside MCP server's tool). */
  | { type: 'approval'; ask: number; tool: string; tier: string; site?: string; summary: string; always: boolean }
  | { type: 'approved'; ask: number; choice: ApprovalChoice }
  | { type: 'tool'; tool: string; state: 'started' | 'done' | 'failed' }
  | { type: 'notice'; text: string }
  | PlayerMessage;

/** How a choice reads on an answered card. */
export const CHOICE_TEXT: Record<ApprovalChoice, string> = {
  deny: 'Denied',
  once: 'Allowed once',
  this_task: 'Allowed for this task',
  always: 'Always allowed',
};

/** Options for one message. */
export interface SendOptions {
  /** Agent mode: tool calls ask first. */
  agent?: boolean;
}

/** The chat's logic, without the webview (so it can be tested). */
export class ChatController {
  private conversation?: string;
  private backendId?: Backend['id'];
  private abort?: AbortController;
  private requestId?: string;
  private video?: string;
  private limits?: VideoLimits;
  /** Clips the chat showed, the only files "Open in system player" opens. */
  readonly clips = new Set<string>();

  constructor(
    private readonly router: Router,
    private readonly post: (m: ViewMessage) => void,
    private readonly exists: (p: string) => boolean = (p) => fs.existsSync(p),
    private readonly folder: () => string | undefined = () => undefined,
  ) {}

  /** Approval cards of the running request still waiting for an answer. */
  readonly waiting = new Set<number>();

  /** Answers approval card `ask` of the running request. */
  async approve(ask: number, choice: ApprovalChoice): Promise<boolean> {
    const id = this.requestId;
    if (!id || !this.waiting.has(ask)) {
      return false;
    }
    this.waiting.delete(ask);
    try {
      await this.router.bragi.approve(id, ask, choice);
    } catch (e) {
      this.post({ type: 'notice', text: `That card couldn't be answered: ${e instanceof Error ? e.message : String(e)}` });
      return false;
    }
    this.post({ type: 'approved', ask, choice });
    return true;
  }

  /** The clip waiting to go with the next message. */
  get pendingVideo(): string | undefined {
    return this.video;
  }

  /** The model's video limits as last read (for the limits box). */
  get videoLimits(): VideoLimits | undefined {
    return this.limits;
  }

  /**
   * Holds the clip at `file` for the next message and shows the model's video limits chip.
   * Refuses what isn't a clip on disk; the direct target can't take one.
   */
  async attachVideo(file: string): Promise<void> {
    if (!isVideoPath(file)) {
      throw new BragiError(`${path.basename(file)} isn't a video (${VIDEO_EXTENSIONS.join(', ')})`, 'bad_request');
    }
    if (!path.isAbsolute(file) || !this.exists(file)) {
      throw new BragiError(`${file} isn't there`, 'bad_request');
    }
    if (this.router.target() === 'direct') {
      throw new BragiError('A video needs the Bragi target: bragi serve samples the clip for the model.', 'bad_request');
    }
    this.video = file;
    this.post({ type: 'attached', name: path.basename(file) });
    await this.refreshVideoLimits();
  }

  detachVideo(): void {
    this.video = undefined;
    this.post({ type: 'detached' });
  }

  /** Reads the current model's video limits and redraws the chip. */
  async refreshVideoLimits(): Promise<VideoLimits | undefined> {
    if (!this.video) {
      return undefined;
    }
    const name = path.basename(this.video);
    try {
      this.limits = await this.router.bragi.videoLimits(this.router.modelFor(this.router.bragi));
      this.post({ type: 'attached', name, chip: this.limits.chip, ready: this.limits.ready });
    } catch (e) {
      this.post({ type: 'attached', name, chip: e instanceof Error ? e.message : String(e), ready: false });
    }
    return this.limits;
  }

  /** Sets this device's video limits for the current model (`key=value …`; empty clears them). */
  async setVideoLimits(text: string): Promise<VideoLimits> {
    const model = this.limits?.model ?? (await this.router.bragi.videoLimits(this.router.modelFor(this.router.bragi))).model;
    this.limits = await this.router.bragi.setVideoLimits(model, text.trim());
    if (this.video) {
      this.post({ type: 'attached', name: path.basename(this.video), chip: this.limits.chip, ready: this.limits.ready });
    }
    return this.limits;
  }

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
  async send(prompt: string, context?: EditorContext, options: SendOptions = {}): Promise<string> {
    prompt = prompt.trim();
    if (!prompt || this.busy) {
      return '';
    }
    const video = this.video;
    const parts = [context ? (context.selection ? `selection in ${context.path}` : context.path) : undefined, video ? path.basename(video) : undefined].filter(Boolean);
    const label = parts.length ? parts.join(' and ') : undefined;
    this.post({ type: 'user', text: prompt, context: label });
    if (video) {
      this.video = undefined;
      this.post({ type: 'detached' });
    }
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
          case 'video': {
            const m = playerMessage(e as unknown as VideoEvent, this.exists);
            if (m.clip) {
              this.clips.add(m.clip);
            }
            this.post(m);
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
          case 'approval': {
            const a = e as { ask: number; tool: string; tier: string; site?: string | null; summary: string; always?: boolean };
            this.waiting.add(a.ask);
            this.post({ type: 'approval', ask: a.ask, tool: a.tool, tier: a.tier, site: a.site ?? undefined, summary: a.summary, always: a.always === true });
            break;
          }
          case 'tool': {
            const t = e as { tool: string; state: 'started' | 'done' | 'failed' };
            this.post({ type: 'tool', tool: t.tool, state: t.state });
            break;
          }
          case 'notice':
            this.post({ type: 'notice', text: (e as { text: string }).text });
            break;
          case 'error': {
            const er = e as { kind: string; message: string };
            throw new BragiError(er.message, er.kind);
          }
          default:
            break;
        }
      };
      await backend.chat({ prompt, context, conversation: this.conversation, model: this.router.modelFor(backend), video: video ? { path: video } : undefined, agent: options.agent || undefined, folder: options.agent ? this.folder() : undefined }, onEvent, abort.signal);
      this.post({ type: 'done' });
    } catch (e) {
      const err = e instanceof BragiError ? e : new BragiError(e instanceof Error ? e.message : String(e), e instanceof Error && e.name === 'AbortError' ? 'cancelled' : 'other');
      this.post(err.kind === 'cancelled' ? { type: 'done' } : { type: 'error', message: err.message });
    } finally {
      if (this.abort === abort) {
        this.abort = undefined;
      }
      this.requestId = undefined;
      this.waiting.clear();
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
  /** Folders of clips the panel plays (it may read only these and its own media). */
  private roots = new Set<string>();
  readonly controller: ChatController;

  constructor(
    private readonly extensionUri: vscode.Uri,
    private readonly routes: Router,
    private readonly targetLabel: () => string,
  ) {
    this.controller = new ChatController(
      routes,
      (m) => this.post(m),
      (p) => fs.existsSync(p),
      () => vscode.workspace.workspaceFolders?.[0]?.uri.fsPath,
    );
  }

  private router() {
    return this.routes.bragi;
  }

  private post(m: ViewMessage): void {
    if (this.view) {
      void this.view.webview.postMessage(this.deliverable(this.view, m));
    } else {
      this.queued.push(m);
    }
  }

  /** `m` as the webview gets it: a clip it plays gets a webview URI, and its folder is let in. */
  private deliverable(view: vscode.WebviewView, m: ViewMessage): ViewMessage {
    if (m.type !== 'video') {
      return m;
    }
    return withSource(m, (clip) => {
      if (!this.roots.has(clip)) {
        this.roots.add(clip);
        view.webview.options = { enableScripts: true, localResourceRoots: this.localRoots() };
      }
      return view.webview.asWebviewUri(vscode.Uri.file(clip)).toString();
    });
  }

  private localRoots(): vscode.Uri[] {
    return resourceRoots(vscode.Uri.joinPath(this.extensionUri, 'media').fsPath, this.roots).map((p) => vscode.Uri.file(p));
  }

  /** Asks for a clip with the file dialog and holds it for the next message. */
  async pickVideo(): Promise<void> {
    const picked = await vscode.window.showOpenDialog({ canSelectMany: false, openLabel: 'Attach', title: 'Attach a video to the chat', filters: { Videos: VIDEO_EXTENSIONS } });
    if (picked?.[0]) {
      await this.attach(picked[0].fsPath);
    }
  }

  private async attach(file: string): Promise<void> {
    try {
      await this.controller.attachVideo(file);
    } catch (e) {
      void vscode.window.showErrorMessage(e instanceof Error ? e.message : String(e));
    }
  }

  /** Sets this device's video limits for the current model in an input box. */
  async editVideoLimits(): Promise<void> {
    try {
      const now = this.controller.videoLimits ?? (await this.controller.refreshVideoLimits()) ?? (await this.router().videoLimits());
      const value = await vscode.window.showInputBox({
        title: `Video limits for ${now.model}`,
        prompt: `Reported: ${now.reported || 'nothing'}. Fills what isn't reported and only lowers what is; empty clears. Keys: ${now.keys.map((k) => k.key).join(', ')}`,
        value: now.user,
        placeHolder: 'image_tokens=258 max_images=32',
      });
      if (value === undefined) {
        return;
      }
      const after = await this.controller.setVideoLimits(value);
      const ignored = after.ignored?.length ? ` Ignored (they would raise a reported limit): ${after.ignored.join(', ')}.` : '';
      void vscode.window.showInformationMessage(`${after.chip}.${ignored}`);
    } catch (e) {
      void vscode.window.showErrorMessage(e instanceof Error ? e.message : String(e));
    }
  }

  /** Opens a clip the chat showed in the system player. */
  async openClip(file: string): Promise<boolean> {
    if (!this.controller.clips.has(file) || !fs.existsSync(file)) {
      void vscode.window.showErrorMessage("That clip isn't on this machine any more.");
      return false;
    }
    return vscode.env.openExternal(vscode.Uri.file(file));
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
    view.webview.options = { enableScripts: true, localResourceRoots: this.localRoots() };
    const n = nonce();
    const script = view.webview.asWebviewUri(vscode.Uri.joinPath(media, 'chat.js'));
    const style = view.webview.asWebviewUri(vscode.Uri.joinPath(media, 'chat.css'));
    view.webview.html = `<!DOCTYPE html>
<html lang="en"><head><meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${view.webview.cspSource}; script-src 'nonce-${n}'; media-src ${view.webview.cspSource}; img-src data:;">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<link rel="stylesheet" href="${style}"><title>Bragi</title></head>
<body>
<header><button id="target" title="Choose target and model"></button><button id="new" title="New chat">New</button></header>
<main id="log" aria-live="polite"></main>
<footer>
<textarea id="input" rows="3" placeholder="Ask about your code (Enter sends, Shift+Enter is a new line)"></textarea>
<div id="clip" class="row clip" hidden><span id="clip-name"></span><span id="clip-chip" class="meta grow"></span><button id="clip-limits" title="Set this model's video limits">Limits…</button><button id="clip-remove" title="Don't send the clip">✕</button></div>
<div class="row"><label><input type="checkbox" id="ctx" checked> Include editor context</label><label title="The model may use tools (a node's, or your MCP servers'); each call asks first"><input type="checkbox" id="agent"> Agent</label><button id="video" title="Attach a video (or drop one here)">Video…</button>
<span class="grow"></span><button id="stop" hidden>Stop</button><button id="send">Send</button></div>
</footer>
<script nonce="${n}" src="${script}"></script>
</body></html>`;
    view.webview.onDidReceiveMessage(async (m: { type: string; prompt?: string; context?: boolean; agent?: boolean; code?: string; uris?: string; path?: string; ask?: number; choice?: ApprovalChoice }) => {
      switch (m.type) {
        case 'ready':
          this.refreshTarget();
          for (const q of this.queued.splice(0)) {
            void view.webview.postMessage(this.deliverable(view, q));
          }
          break;
        case 'attachVideo':
          await this.pickVideo();
          break;
        case 'dropVideo': {
          const file = droppedClip(m.uris ?? '');
          if (file) {
            await this.attach(file);
          } else {
            void vscode.window.showErrorMessage(`Drop a video file (${VIDEO_EXTENSIONS.join(', ')}). Hold Shift while dropping from the Explorer.`);
          }
          break;
        }
        case 'detachVideo':
          this.controller.detachVideo();
          break;
        case 'videoLimits':
          await this.editVideoLimits();
          break;
        case 'openClip':
          if (m.path) {
            await this.openClip(m.path);
          }
          break;
        case 'send': {
          const editor = vscode.window.activeTextEditor;
          const ctx = m.context && editor ? editorContext(editor) : undefined;
          await this.controller.send(m.prompt ?? '', ctx, { agent: m.agent === true });
          break;
        }
        case 'approve':
          if (m.ask !== undefined && m.choice && m.choice in CHOICE_TEXT) {
            await this.controller.approve(m.ask, m.choice);
          }
          break;
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
