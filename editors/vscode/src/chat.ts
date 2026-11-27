// The chat panel: a conversation with editor context, kept in Bragi's history on the Bragi target.

import * as fs from 'fs';
import * as path from 'path';
import * as vscode from 'vscode';
import { ApprovalChoice, Backend, Router } from './backends';
import { editorContext } from './context';
import { ChatFile, MEDIA_EXTENSIONS, readChatFile, selectionFile } from './files';
import { Manage, StoredAttachment, StoredMessage } from './manage';
import { BragiError, ChatEvent, EditorContext, VideoLimits } from './types';
import { droppedClip, droppedFiles, isVideoPath, PlayerMessage, playerMessage, resourceRoots, VIDEO_EXTENSIONS, VideoEvent, withSource } from './video';

/** A file in a message as the panel draws it: an image or sound plays from `src` (a `data:` URL). */
export interface ViewFile {
  name: string;
  kind: 'text' | 'image' | 'audio' | 'video';
  src?: string;
}

/** What the panel shows; the controller sends these. */
export type ViewMessage =
  | { type: 'user'; text: string; context?: string; files?: ViewFile[] }
  /** Files waiting to go with the next message. */
  | { type: 'files'; files: ViewFile[] }
  /** The ids Bragi gave the last question and answer (reactions and delete need them). */
  | { type: 'ids'; user?: string; answer?: string }
  /** A message of an opened chat. */
  | { type: 'message'; role: 'user' | 'assistant'; id: string; text: string; files?: ViewFile[]; reactions?: string[]; model?: string }
  | { type: 'reactions'; id: string; reactions: string[] }
  | { type: 'removed'; ids: string[] }
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
  /** Files and selections waiting to go with the next message. */
  private files: ChatFile[] = [];
  private readonly manage: Manage;

  constructor(
    private readonly router: Router,
    private readonly post: (m: ViewMessage) => void,
    private readonly exists: (p: string) => boolean = (p) => fs.existsSync(p),
    private readonly folder: () => string | undefined = () => undefined,
  ) {
    this.manage = new Manage(() => router.bragi);
  }

  /** The files waiting to go with the next message. */
  get pendingFiles(): readonly ChatFile[] {
    return this.files;
  }

  /** Holds `file` (text, an image or sound) for the next message. */
  attachFile(file: string): ChatFile {
    const f = readChatFile(file);
    this.addFile(f);
    return f;
  }

  /** Holds the selection (`relPath`, 1-based lines) for the next message. */
  attachSelection(relPath: string, startLine: number, endLine: number, text: string): ChatFile {
    const f = selectionFile(relPath, startLine, endLine, text);
    this.addFile(f);
    return f;
  }

  private addFile(f: ChatFile): void {
    if (f.kind !== 'text' && this.router.target() === 'direct') {
      throw new BragiError('Images and sound need the Bragi target (the direct server gets text only).', 'bad_request');
    }
    this.files = [...this.files.filter((x) => x.name !== f.name), f];
    this.postFiles();
  }

  detachFile(index: number): void {
    this.files = this.files.filter((_, i) => i !== index);
    this.postFiles();
  }

  private postFiles(): void {
    this.post({ type: 'files', files: this.files.map(viewFile) });
  }

  /** Opens chat `id` from Bragi's history: its messages replace the panel's, and the next message continues it. */
  async open(id: string): Promise<number> {
    this.stop();
    const s = await this.manage.openSession(id);
    this.conversation = s.conversation.id;
    this.backendId = 'bragi';
    this.post({ type: 'reset' });
    for (const m of s.conversation.messages) {
      this.post(historyMessage(m));
    }
    return s.conversation.messages.length;
  }

  private bragiChat(): string {
    if (!this.conversation || this.backendId !== 'bragi') {
      throw new BragiError('Reactions and deleting are for chats kept in Bragi (the Bragi target).', 'bad_request');
    }
    return this.conversation;
  }

  /** Toggles `emoji` on message `id`; returns its reactions. */
  async react(id: string, emoji: string): Promise<string[]> {
    const reactions = await this.manage.react(this.bragiChat(), id, emoji);
    this.post({ type: 'reactions', id, reactions });
    return reactions;
  }

  /** Deletes message `id` (and with `withReply` the answer after it); returns the undo token. */
  async deleteMessage(id: string, withReply: boolean): Promise<string> {
    if (this.busy) {
      throw new BragiError('Stop the answer before deleting it.', 'bad_request');
    }
    const d = await this.manage.deleteMessage(this.bragiChat(), id, withReply);
    this.post({ type: 'removed', ids: d.ids });
    return d.undo;
  }

  /** Puts back the last delete and redraws the chat. */
  async undo(token: string): Promise<void> {
    const id = this.bragiChat();
    await this.manage.undo(token);
    await this.open(id);
  }

  /** After an answer: the ids Bragi gave the question and the answer. */
  private async stamp(): Promise<void> {
    if (!this.conversation || this.backendId !== 'bragi') {
      return;
    }
    try {
      const msgs = (await this.manage.openSession(this.conversation)).conversation.messages;
      const answer = msgs.length && msgs[msgs.length - 1].role === 'assistant' ? msgs[msgs.length - 1] : undefined;
      const user = [...msgs].reverse().find((m) => m.role === 'user');
      this.post({ type: 'ids', user: user?.id, answer: answer?.id });
    } catch {
      // An older Bragi without GET /v1/sessions/{id}: no reactions or delete on this message.
    }
  }

  /** Approval cards of the running request still waiting for an answer. */
  readonly waiting = new Set<number>();

  /** Answers approval card `ask` of the running request. */
  async approve(ask: number, choice: ApprovalChoice): Promise<boolean> {
    const id = this.requestId;
    if (!id || !this.waiting.has(ask)) {
      return false;
    }
    this.waiting.delete(ask);
    // Shown at once: what the answer lets happen streams in while the request is still out.
    this.post({ type: 'approved', ask, choice });
    try {
      await this.router.bragi.approve(id, ask, choice);
    } catch (e) {
      this.post({ type: 'notice', text: `That card couldn't be answered: ${e instanceof Error ? e.message : String(e)}` });
      return false;
    }
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
    this.files = [];
    this.post({ type: 'reset' });
    this.postFiles();
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
    const files = this.files;
    const parts = [context ? (context.selection ? `selection in ${context.path}` : context.path) : undefined, video ? path.basename(video) : undefined].filter(Boolean);
    const label = parts.length ? parts.join(' and ') : undefined;
    this.post({ type: 'user', text: prompt, context: label, files: files.length ? files.map(viewFile) : undefined });
    if (video) {
      this.video = undefined;
      this.post({ type: 'detached' });
    }
    if (files.length) {
      this.files = [];
      this.postFiles();
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
            const a = e as unknown as { ask: number; tool: string; tier: string; site?: string | null; summary: string; always?: boolean };
            this.waiting.add(a.ask);
            this.post({ type: 'approval', ask: a.ask, tool: a.tool, tier: a.tier, site: a.site ?? undefined, summary: a.summary, always: a.always === true });
            break;
          }
          case 'tool': {
            const t = e as unknown as { tool: string; state: 'started' | 'done' | 'failed' };
            this.post({ type: 'tool', tool: t.tool, state: t.state });
            break;
          }
          case 'notice':
            this.post({ type: 'notice', text: (e as unknown as { text: string }).text });
            break;
          case 'error': {
            const er = e as { kind: string; message: string };
            throw new BragiError(er.message, er.kind);
          }
          default:
            break;
        }
      };
      await backend.chat({ prompt, context, conversation: this.conversation, model: this.router.modelFor(backend), video: video ? { path: video } : undefined, agent: options.agent || undefined, folder: options.agent ? this.folder() : undefined, files: files.length ? files : undefined }, onEvent, abort.signal);
      this.post({ type: 'done' });
      await this.stamp();
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

function viewFile(f: ChatFile): ViewFile {
  return { name: f.name, kind: f.kind, src: f.src };
}

function storedFile(a: StoredAttachment): ViewFile {
  const m = a.media;
  if (!m) {
    return { name: a.name, kind: 'text' };
  }
  if (m.kind === 'image') {
    return { name: a.name, kind: 'image', src: `data:${m.mime};base64,${m.data}` };
  }
  if (m.kind === 'audio') {
    return { name: a.name, kind: 'audio', src: `data:audio/${m.format === 'mp3' ? 'mpeg' : 'wav'};base64,${m.data}` };
  }
  return { name: a.name, kind: 'video' };
}

function historyMessage(m: StoredMessage): ViewMessage {
  const files = (m.attachments ?? []).map(storedFile);
  return { type: 'message', role: m.role, id: m.id, text: m.text, files: files.length ? files : undefined, reactions: m.reactions?.length ? m.reactions : undefined, model: m.model ?? undefined };
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

  /** Asks for files with the file dialog and holds them for the next message. */
  async pickFiles(): Promise<void> {
    const picked = await vscode.window.showOpenDialog({
      canSelectMany: true,
      openLabel: 'Attach',
      title: 'Attach files to the chat',
      defaultUri: vscode.workspace.workspaceFolders?.[0]?.uri,
      filters: { 'All files': ['*'], 'Images and sound': MEDIA_EXTENSIONS },
    });
    for (const uri of picked ?? []) {
      this.attachFileShown(uri.fsPath);
    }
  }

  /** Holds `file` for the next message; a problem is shown, not thrown. */
  attachFileShown(file: string): void {
    try {
      this.controller.attachFile(file);
    } catch (e) {
      void vscode.window.showErrorMessage(e instanceof Error ? e.message : String(e));
    }
  }

  /** Holds the active editor's selection for the next message. */
  attachSelection(): boolean {
    const editor = vscode.window.activeTextEditor ?? vscode.window.visibleTextEditors[0];
    if (!editor || editor.selection.isEmpty) {
      void vscode.window.showErrorMessage('Select some text in an editor first.');
      return false;
    }
    const sel = editor.selection;
    const endLine = sel.end.character === 0 && sel.end.line > sel.start.line ? sel.end.line : sel.end.line + 1;
    try {
      this.controller.attachSelection(vscode.workspace.asRelativePath(editor.document.uri), sel.start.line + 1, endLine, editor.document.getText(sel));
      return true;
    } catch (e) {
      void vscode.window.showErrorMessage(e instanceof Error ? e.message : String(e));
      return false;
    }
  }

  /** Deletes a message after asking (a question can take its answer with it); offers Undo. */
  async deleteMessage(id: string, role: 'user' | 'assistant'): Promise<void> {
    const items = role === 'user' ? ['Delete with its answer', 'Delete only this'] : ['Delete'];
    const pick = await vscode.window.showWarningMessage('Delete this message on every linked device?', { modal: true }, ...items);
    if (!pick) {
      return;
    }
    try {
      const undo = await this.controller.deleteMessage(id, pick === 'Delete with its answer');
      if ((await vscode.window.showInformationMessage('Bragi: message deleted.', 'Undo')) === 'Undo') {
        await this.controller.undo(undo);
      }
    } catch (e) {
      void vscode.window.showErrorMessage(e instanceof Error ? e.message : String(e));
    }
  }

  /** Opens chat `id` in the panel. */
  async open(id: string): Promise<number> {
    await vscode.commands.executeCommand(`${ChatView.id}.focus`);
    return this.controller.open(id);
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
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${view.webview.cspSource}; script-src 'nonce-${n}'; media-src ${view.webview.cspSource} data:; img-src data:;">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<link rel="stylesheet" href="${style}"><title>Bragi</title></head>
<body>
<header><button id="target" title="Choose target and model"></button><button id="new" title="New chat">New</button></header>
<main id="log" aria-live="polite"></main>
<footer>
<textarea id="input" rows="3" placeholder="Ask about your code (Enter sends, Shift+Enter is a new line)"></textarea>
<div id="clip" class="row clip" hidden><span id="clip-name"></span><span id="clip-chip" class="meta grow"></span><button id="clip-limits" title="Set this model's video limits">Limits…</button><button id="clip-remove" title="Don't send the clip">✕</button></div>
<div id="files" class="row files" hidden></div>
<div class="row"><label><input type="checkbox" id="ctx" checked> Include editor context</label><label title="The model may use tools (a node's, or your MCP servers'); each call asks first"><input type="checkbox" id="agent"> Agent</label><button id="attach" title="Attach files: text, images (PNG, JPEG, GIF, WebP) or sound (WAV, MP3); or drop them here">File…</button><button id="selection" title="Attach the selected text">Selection</button><button id="video" title="Attach a video (or drop one here)">Video…</button>
<span class="grow"></span><button id="stop" hidden>Stop</button><button id="send">Send</button></div>
</footer>
<script nonce="${n}" src="${script}"></script>
</body></html>`;
    view.webview.onDidReceiveMessage(async (m: { type: string; prompt?: string; context?: boolean; agent?: boolean; code?: string; uris?: string; path?: string; ask?: number; choice?: ApprovalChoice; index?: number; id?: string; emoji?: string; role?: 'user' | 'assistant' }) => {
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
          const others = droppedFiles(m.uris ?? '').filter((f) => f !== file);
          if (file) {
            await this.attach(file);
          }
          for (const f of others) {
            this.attachFileShown(f);
          }
          if (!file && others.length === 0) {
            void vscode.window.showErrorMessage('Drop a file or a video. Hold Shift while dropping from the Explorer.');
          }
          break;
        }
        case 'attachFile':
          await this.pickFiles();
          break;
        case 'attachSelection':
          this.attachSelection();
          break;
        case 'detachFile':
          if (typeof m.index === 'number') {
            this.controller.detachFile(m.index);
          }
          break;
        case 'react':
          if (m.id && m.emoji) {
            try {
              await this.controller.react(m.id, m.emoji);
            } catch (e) {
              void vscode.window.showErrorMessage(e instanceof Error ? e.message : String(e));
            }
          }
          break;
        case 'delete':
          if (m.id) {
            await this.deleteMessage(m.id, m.role === 'user' ? 'user' : 'assistant');
          }
          break;
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
