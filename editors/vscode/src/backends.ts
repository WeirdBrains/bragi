// Where answers come from: the local Bragi API (`bragi serve`) or, directly, an OpenAI-compatible
// server. Both speak the same Backend interface; Router picks one from the settings.

import * as vscode from 'vscode';
import { chatContextText, cleanSuggestion, effectiveScope, PromptMessage, rewritePrompt, rewriteResult, suggestPrompt } from './prompts';
import { Applied, applyHunks, computeHunks, HunkList } from './hunks';
import { Secrets } from './tokens';
import { BragiError, BragiTarget, ChatEvent, EditorContext, ModelList, RewriteResult, Scope, TargetOption, VideoAttach, VideoLimits } from './types';

export interface ChatTurn {
  prompt: string;
  context?: EditorContext;
  conversation?: string;
  model?: string;
  /** A clip to go with the turn (Bragi target only). */
  video?: VideoAttach;
}

export interface Backend {
  readonly id: 'bragi' | 'direct';
  readonly label: string;
  models(signal?: AbortSignal): Promise<ModelList>;
  suggest(ctx: EditorContext, model: string | undefined, signal?: AbortSignal): Promise<string>;
  rewrite(ctx: EditorContext, instruction: string, scope: Scope, model: string | undefined, signal?: AbortSignal): Promise<RewriteResult>;
  /** The hunks from `oldText` (the document) to `newText` (a rewrite), to review one at a time. */
  hunks(oldText: string, newText: string, signal?: AbortSignal): Promise<HunkList>;
  /** `oldText` with only the accepted hunks of `oldText` → `newText`. */
  applyHunks(oldText: string, newText: string, accepted: boolean[], signal?: AbortSignal): Promise<Applied>;
  /** Streams a chat turn; returns the conversation it is in. */
  chat(turn: ChatTurn, onEvent: (e: ChatEvent) => void, signal?: AbortSignal): Promise<string | undefined>;
}

/** Calls `onData` with each `data:` payload of a server-sent event stream. */
export async function readSse(res: Response, onData: (data: string) => void): Promise<void> {
  if (!res.body) {
    return;
  }
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buf = '';
  for (;;) {
    const { done, value } = await reader.read();
    if (done) {
      break;
    }
    buf += decoder.decode(value, { stream: true }).replace(/\r\n/g, '\n');
    let i: number;
    while ((i = buf.indexOf('\n\n')) >= 0) {
      const event = buf.slice(0, i);
      buf = buf.slice(i + 2);
      const data = event
        .split('\n')
        .filter((l) => l.startsWith('data:'))
        .map((l) => l.slice(5).replace(/^ /, ''))
        .join('\n');
      if (data) {
        onData(data);
      }
    }
  }
}

async function failure(res: Response, what: string): Promise<BragiError> {
  let message = `${what}: HTTP ${res.status}`;
  let kind = 'other';
  try {
    const body = (await res.json()) as { error?: { message?: string; kind?: string; type?: string } };
    if (body.error?.message) {
      message = body.error.message;
      kind = body.error.kind ?? body.error.type ?? kind;
    }
  } catch {
    // Not JSON: keep the status.
  }
  if (res.status === 401) {
    kind = 'unauthorized';
  }
  return new BragiError(message, kind, res.status);
}

function unreachable(what: string, e: unknown): BragiError {
  if (e instanceof BragiError) {
    return e;
  }
  if (e instanceof Error && e.name === 'AbortError') {
    return new BragiError('cancelled', 'cancelled');
  }
  return new BragiError(`${what} is not reachable (${e instanceof Error ? e.message : String(e)})`, 'unreachable');
}

/** The local Bragi app's API. */
export class BragiBackend implements Backend {
  readonly id = 'bragi' as const;
  readonly label = 'Bragi';

  /** Called when the API doesn't answer: true if it was started (the request is tried once more). */
  revive?: () => Promise<boolean>;

  constructor(
    private readonly baseUrl: () => string,
    private readonly secrets: Secrets,
    private readonly target: () => BragiTarget | undefined = () => undefined,
  ) {}

  /** A JSON call to `bragi serve` (the management routes: links, endpoints, sessions, MCP servers). */
  async api<T = unknown>(method: string, path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
    return (await (await this.request(method, path, body, signal)).json()) as T;
  }

  /** Where requests go: the pool and the endpoints Bragi's Settings offers alongside it. */
  async targets(signal?: AbortSignal): Promise<TargetOption[]> {
    const r = (await (await this.request('GET', '/v1/targets', undefined, signal)).json()) as { targets: TargetOption[] };
    return r.targets;
  }

  private url(path: string): string {
    const base = this.baseUrl().replace(/\/+$/, '');
    const u = new URL(base + path);
    if (!['127.0.0.1', 'localhost', '[::1]', '::1'].includes(u.hostname)) {
      throw new BragiError(`bragi.serveUrl must be a loopback address, not ${u.hostname}`, 'bad_request');
    }
    return u.toString();
  }

  private async headers(): Promise<Record<string, string>> {
    let token = await this.secrets.serveToken();
    if (!token && (await this.secrets.importServeToken())) {
      token = await this.secrets.serveToken();
    }
    if (!token) {
      throw new BragiError('No Bragi API token yet. Run "Bragi: Import Token from the Bragi App" or "Bragi: Set Bragi API Token".', 'unauthorized');
    }
    return { authorization: `Bearer ${token}`, 'content-type': 'application/json' };
  }

  private async request(method: string, path: string, body: unknown, signal?: AbortSignal): Promise<Response> {
    const url = this.url(path);
    let headers = await this.headers();
    let res: Response;
    try {
      try {
        res = await fetch(url, { method, headers, body: body === undefined ? undefined : JSON.stringify(body), signal });
      } catch (e) {
        if (!this.revive || (e instanceof Error && e.name === 'AbortError') || !(await this.revive())) {
          throw e;
        }
        headers = await this.headers();
        res = await fetch(url, { method, headers, body: body === undefined ? undefined : JSON.stringify(body), signal });
      }
    } catch (e) {
      throw unreachable('The local Bragi API', e);
    }
    if (!res.ok) {
      throw await failure(res, path);
    }
    return res;
  }

  async cancel(requestId: string): Promise<void> {
    await this.request('POST', '/v1/cancel', { request_id: requestId });
  }

  async health(signal?: AbortSignal): Promise<boolean> {
    try {
      const res = await fetch(this.url('/v1/health'), { signal });
      return res.ok;
    } catch {
      return false;
    }
  }

  async models(signal?: AbortSignal): Promise<ModelList> {
    const s = (await (await this.request('GET', '/v1/state', undefined, signal)).json()) as {
      models: { id: string; status: string }[];
      default_model?: string | null;
      message?: string | null;
    };
    return { models: s.models.map((m) => ({ id: m.id, status: m.status })), defaultModel: s.default_model ?? undefined, message: s.message ?? undefined };
  }

  async suggest(ctx: EditorContext, model: string | undefined, signal?: AbortSignal): Promise<string> {
    const r = (await (await this.request('POST', '/v1/code/suggest', { context: ctx, model: model || undefined, target: this.target() }, signal)).json()) as { text: string };
    return r.text;
  }

  async rewrite(ctx: EditorContext, instruction: string, scope: Scope, model: string | undefined, signal?: AbortSignal): Promise<RewriteResult> {
    const r = (await (await this.request('POST', '/v1/code/rewrite', { context: ctx, instruction, scope, model: model || undefined, target: this.target() }, signal)).json()) as {
      code: string;
      scope: Scope;
      model?: string;
    };
    return { code: r.code, scope: r.scope, model: r.model };
  }

  /** bragi-core's hunks (`code::FileDiff`), the same the terminal app reviews. */
  async hunks(oldText: string, newText: string, signal?: AbortSignal): Promise<HunkList> {
    return (await (await this.request('POST', '/v1/code/hunks', { old: oldText, new: newText }, signal)).json()) as HunkList;
  }

  async applyHunks(oldText: string, newText: string, accepted: boolean[], signal?: AbortSignal): Promise<Applied> {
    return (await (await this.request('POST', '/v1/code/apply', { old: oldText, new: newText, accepted }, signal)).json()) as Applied;
  }

  async chat(turn: ChatTurn, onEvent: (e: ChatEvent) => void, signal?: AbortSignal): Promise<string | undefined> {
    const res = await this.request(
      'POST',
      '/v1/code/chat',
      { prompt: turn.prompt, context: turn.context, conversation: turn.conversation, model: turn.model || undefined, target: this.target(), video: turn.video },
      signal,
    );
    let conversation: string | undefined;
    await readSse(res, (data) => {
      const e = JSON.parse(data) as ChatEvent;
      if (e.type === 'conversation') {
        conversation = (e as { id: string }).id;
      }
      onEvent(e);
    });
    return conversation;
  }

  /** `model`'s video limits at the current target (its default model when absent). */
  videoLimits(model?: string, signal?: AbortSignal): Promise<VideoLimits> {
    return this.api<VideoLimits>('POST', '/v1/video/limits', { model: model || undefined, target: this.target() }, signal);
  }

  /** Sets this device's video limits for `model` (`key=value …`; empty clears them). */
  setVideoLimits(model: string, limits: string, signal?: AbortSignal): Promise<VideoLimits> {
    return this.api<VideoLimits>('POST', '/v1/video/limits/set', { model, limits, target: this.target() }, signal);
  }
}

/** An OpenAI-compatible server, called directly (Ymir's bridge, llama.cpp, Ollama, …). */
export class DirectBackend implements Backend {
  readonly id = 'direct' as const;
  readonly label = 'Direct';
  /** Direct chats keep their history here (the server keeps none). */
  private readonly chats = new Map<string, PromptMessage[]>();
  private next = 1;

  constructor(
    private readonly config: () => { url: string; model: string },
    private readonly secrets: Secrets,
  ) {}

  private async post(path: string, body: unknown, signal?: AbortSignal): Promise<Response> {
    const { url } = this.config();
    if (!url) {
      throw new BragiError('Set bragi.direct.url to an OpenAI-compatible server first.', 'bad_request');
    }
    const key = await this.secrets.directKey();
    const headers: Record<string, string> = { 'content-type': 'application/json' };
    if (key) {
      headers.authorization = `Bearer ${key}`;
    }
    let res: Response;
    try {
      res = await fetch(url.replace(/\/+$/, '') + path, { method: body === undefined ? 'GET' : 'POST', headers, body: body === undefined ? undefined : JSON.stringify(body), signal });
    } catch (e) {
      throw unreachable(`The server at ${url}`, e);
    }
    if (!res.ok) {
      throw await failure(res, path);
    }
    return res;
  }

  private async model(model: string | undefined, signal?: AbortSignal): Promise<string> {
    const m = model || this.config().model;
    if (m) {
      return m;
    }
    const list = await this.models(signal);
    if (!list.defaultModel) {
      throw new BragiError('The direct server lists no models.', 'unavailable');
    }
    return list.defaultModel;
  }

  async models(signal?: AbortSignal): Promise<ModelList> {
    const r = (await (await this.post('/models', undefined, signal)).json()) as { data?: { id: string }[] };
    const models = (r.data ?? []).map((m) => ({ id: m.id }));
    return { models, defaultModel: this.config().model || models[0]?.id };
  }

  private async complete(messages: PromptMessage[], model: string | undefined, signal?: AbortSignal): Promise<string> {
    const body = { model: await this.model(model, signal), messages, stream: false };
    const r = (await (await this.post('/chat/completions', body, signal)).json()) as { choices?: { message?: { content?: string } }[] };
    return r.choices?.[0]?.message?.content ?? '';
  }

  async suggest(ctx: EditorContext, model: string | undefined, signal?: AbortSignal): Promise<string> {
    return cleanSuggestion(await this.complete(suggestPrompt(ctx), model, signal), ctx);
  }

  async rewrite(ctx: EditorContext, instruction: string, scope: Scope, model: string | undefined, signal?: AbortSignal): Promise<RewriteResult> {
    const s = effectiveScope(ctx, scope);
    const answer = await this.complete(rewritePrompt(ctx, instruction, s), model, signal);
    const original = s === 'selection' ? ctx.selection : ctx.before + ctx.selection + ctx.after;
    return { code: rewriteResult(answer, original), scope: s };
  }

  // No Bragi behind the direct target: the same hunks, worked out here.
  async hunks(oldText: string, newText: string): Promise<HunkList> {
    return computeHunks(oldText, newText);
  }

  async applyHunks(oldText: string, newText: string, accepted: boolean[]): Promise<Applied> {
    return applyHunks(oldText, newText, accepted);
  }

  async chat(turn: ChatTurn, onEvent: (e: ChatEvent) => void, signal?: AbortSignal): Promise<string | undefined> {
    if (turn.video) {
      throw new BragiError('A video needs the Bragi target: bragi serve samples the clip for the model (the direct server gets text only).', 'bad_request');
    }
    const id = turn.conversation && this.chats.has(turn.conversation) ? turn.conversation : `direct-${this.next++}`;
    const history = this.chats.get(id) ?? [];
    const content = turn.context ? `${chatContextText(turn.context)}${turn.prompt}` : turn.prompt;
    const messages = [...history, { role: 'user' as const, content }];
    onEvent({ type: 'conversation', id, request_id: id });
    const model = await this.model(turn.model, signal);
    onEvent({ type: 'started', node: this.config().url, model });
    const res = await this.post('/chat/completions', { model, messages, stream: true }, signal);
    let text = '';
    await readSse(res, (data) => {
      if (data === '[DONE]') {
        return;
      }
      const chunk = JSON.parse(data) as { choices?: { delta?: { content?: string } }[]; error?: { message?: string } };
      if (chunk.error) {
        throw new BragiError(chunk.error.message ?? 'the server failed', 'other');
      }
      const piece = chunk.choices?.[0]?.delta?.content;
      if (piece) {
        onEvent({ type: 'delta', offset: text.length, text: piece });
        text += piece;
      }
    });
    this.chats.set(id, [...messages, { role: 'assistant', content: text }]);
    onEvent({ type: 'done', finish_reason: 'stop' });
    return id;
  }
}

/** Picks the backend from the settings, falling back to the direct server when asked to. */
export class Router {
  readonly bragi: BragiBackend;
  readonly direct: DirectBackend;

  constructor(secrets: Secrets) {
    const cfg = () => vscode.workspace.getConfiguration('bragi');
    this.bragi = new BragiBackend(
      () => cfg().get('serveUrl', 'http://127.0.0.1:4877'),
      secrets,
      () => cfg().get<BragiTarget | null>('bragiTarget', null) ?? undefined,
    );
    this.direct = new DirectBackend(() => ({ url: cfg().get('direct.url', ''), model: cfg().get('direct.model', '') }), secrets);
  }

  target(): 'bragi' | 'direct' {
    return vscode.workspace.getConfiguration('bragi').get<string>('target', 'bragi') === 'direct' ? 'direct' : 'bragi';
  }

  /** The backend to use now. */
  async backend(signal?: AbortSignal): Promise<Backend> {
    if (this.target() === 'direct') {
      return this.direct;
    }
    const cfg = vscode.workspace.getConfiguration('bragi');
    if (cfg.get('fallbackToDirect', false) && cfg.get('direct.url', '') && !(await this.bragi.health(signal))) {
      return this.direct;
    }
    return this.bragi;
  }

  /** The model to pass to `backend` (a Bragi model name means nothing to the direct server). */
  modelFor(backend: Backend): string | undefined {
    const cfg = vscode.workspace.getConfiguration('bragi');
    return (backend.id === 'direct' ? cfg.get<string>('direct.model', '') : cfg.get<string>('model', '')) || undefined;
  }
}
