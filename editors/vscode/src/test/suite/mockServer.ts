// A stand-in for `bragi serve` and for an OpenAI-compatible server, on 127.0.0.1.

import * as http from 'http';
import { AddressInfo } from 'net';
import { applyHunks, computeHunks } from '../../hunks';

export interface Seen {
  method: string;
  path: string;
  auth?: string;
  body: any;
}

export const TOKEN = 'f'.repeat(64);

export class MockServer {
  readonly seen: Seen[] = [];
  private server!: http.Server;
  port = 0;
  suggestion = 'a + b';
  rewrite = (code: string) => code.toUpperCase();
  // The management routes' state (what `bragi serve` keeps in Bragi's home).
  links: { address: string; peer: string; online: boolean }[] = [];
  endpoints: any[] = [];
  mcpServers: any[] = [];
  /** False: /v1/mcp-servers answers 404, like a Bragi without the MCP client. */
  mcpClient = true;
  linked: boolean | undefined;
  /** The video override set through /v1/video/limits/set. */
  videoUser = '';
  /** Answers to agent approval cards, in order. */
  approvals: { request_id: string; ask: number; choice: string }[] = [];
  /** An agent chat waiting at its card: called with the choice. */
  private waitingCard?: (choice: string) => void;
  private nextId = 1;
  /** Chats kept by the mock (`/v1/code/chat` adds to them): id → messages. */
  chats = new Map<string, any[]>();
  sessionFolders = new Map<string, string>();
  folders: { id: string; name: string; parent?: string | null }[] = [];
  private lastDelete?: { chat: string; removed: { at: number; m: any }[] };
  /** Linking as `GET /v1/state` reports it (bragi-core's PairState). */
  pair: any = { state: 'linked', user: 'u-1', device: 'This Bragi' };
  /** What the other device does with a link code: `match` shows a code to compare. */
  offerScript: 'match' | 'wait' = 'match';

  get url(): string {
    return `http://127.0.0.1:${this.port}`;
  }

  async start(): Promise<void> {
    this.server = http.createServer((req, res) => {
      let raw = '';
      req.on('data', (c) => (raw += c));
      req.on('end', () => this.handle(req, res, raw));
    });
    await new Promise<void>((r) => this.server.listen(0, '127.0.0.1', r));
    this.port = (this.server.address() as AddressInfo).port;
  }

  stop(): Promise<void> {
    return new Promise((r) => this.server.close(() => r()));
  }

  last(path: string): Seen | undefined {
    return [...this.seen].reverse().find((s) => s.path === path);
  }

  private json(res: http.ServerResponse, status: number, body: unknown): void {
    res.writeHead(status, { 'content-type': 'application/json' });
    res.end(JSON.stringify(body));
  }

  /** A server as bragi-core's McpServerView draws it (no secrets). */
  private mcpView(body: any, old?: any): any {
    return {
      name: body.name,
      transport: body.transport,
      enabled: body.enabled ?? true,
      target: body.transport === 'stdio' ? [body.command, ...(body.args ?? [])].join(' ') : body.url,
      env_names: Object.keys(body.secrets?.env ?? {}),
      header_names: Object.keys(body.secrets?.headers ?? {}),
      has_bearer: body.secrets ? !!body.secrets.bearer : (old?.has_bearer ?? false),
      scope: body.scope === 'folder' ? { kind: 'folder', value: body.folder } : { kind: 'all' },
      always_allow: old?.always_allow ?? [],
      warning: body.transport === 'stdio' ? 'runs as you, with your permissions' : null,
    };
  }

  /** An agent turn: some text, a card for `docs.search`, then what the answer let happen. */
  private agentChat(res: http.ServerResponse, body: any): void {
    const send = (e: unknown) => res.write(`data: ${JSON.stringify(e)}\n\n`);
    send({ type: 'conversation', id: body.conversation ?? 'c-agent', request_id: 'r-agent' });
    send({ type: 'started', node: 'node-1', model: 'gemma-4-e4b' });
    send({ type: 'delta', offset: 0, text: 'Let me look. ' });
    send({ type: 'approval', ask: 1, tool: 'docs.search', tier: 'read', site: null, summary: 'Search the docs for "add"', always: true });
    this.waitingCard = (choice) => {
      if (choice === 'deny') {
        send({ type: 'notice', text: 'docs.search was declined' });
        send({ type: 'delta', offset: 13, text: 'I could not search.' });
      } else {
        if (choice === 'always') {
          const s = this.mcpServers[0];
          if (s) {
            s.always_allow = [...s.always_allow, 'docs.search'];
          }
        }
        send({ type: 'tool', call_id: 't1', tool: 'docs.search', state: 'started' });
        send({ type: 'tool', call_id: 't1', tool: 'docs.search', state: 'done' });
        send({ type: 'delta', offset: 13, text: 'The docs say add sums.' });
      }
      send({ type: 'done', finish_reason: 'stop' });
      res.end();
    };
  }

  /** Keeps a plain chat turn (with its attachments) the way Bragi's history would. */
  private record(body: any, id: string, answer: string): void {
    const msgs = this.chats.get(id) ?? [];
    const n = this.nextId++;
    const attachments = (body.attachments ?? []).map((a: any) => {
      if (a.text !== undefined) {
        return { name: a.name, text: a.text };
      }
      const ext = String(a.name).split('.').pop()!.toLowerCase();
      return ['wav', 'mp3'].includes(ext) ? { name: a.name, text: '', media: { kind: 'audio', format: ext, data: a.data } } : { name: a.name, text: '', media: { kind: 'image', mime: `image/${ext === 'jpg' ? 'jpeg' : ext}`, data: a.data } };
    });
    msgs.push({ id: `u${n}`, role: 'user', text: body.prompt, state: 'complete', attachments, at_ms: n });
    msgs.push({ id: `a${n}`, role: 'assistant', text: answer, state: 'complete', model: 'gemma-4-e4b', at_ms: n });
    this.chats.set(id, msgs);
  }

  private chatRoutes(req: http.IncomingMessage, res: http.ServerResponse, path: string, body: any): boolean {
    const notFound = (what: string) => this.json(res, 404, { error: { kind: 'not_found', message: `no ${what}` } });
    let m = /^\/v1\/sessions\/([^/]+)\/messages\/([^/]+)(\/react)?$/.exec(path);
    if (m) {
      const msgs = this.chats.get(m[1]);
      const at = msgs?.findIndex((x) => x.id === m![2]) ?? -1;
      if (!msgs || at < 0) {
        notFound('message');
        return true;
      }
      if (m[3]) {
        const r: string[] = msgs[at].reactions ?? [];
        msgs[at].reactions = r.includes(body.emoji) ? r.filter((e) => e !== body.emoji) : [...r, body.emoji];
        this.json(res, 200, { reactions: msgs[at].reactions });
        return true;
      }
      const withReply = (req.url ?? '').includes('with_reply=true') && msgs[at + 1]?.role === 'assistant';
      const removed = msgs.splice(at, withReply ? 2 : 1).map((x, i) => ({ at: at + i, m: x }));
      this.lastDelete = { chat: m[1], removed };
      this.json(res, 200, { undo: 'undo-1', messages: removed.length, ids: removed.map((r) => r.m.id) });
      return true;
    }
    m = /^\/v1\/sessions\/([^/]+)(\/folder)?$/.exec(path);
    if (m && (req.method === 'GET' || m[2])) {
      const msgs = this.chats.get(m[1]);
      if (!msgs) {
        notFound('chat');
        return true;
      }
      if (m[2]) {
        if (body.folder) {
          this.sessionFolders.set(m[1], body.folder);
        } else {
          this.sessionFolders.delete(m[1]);
        }
        this.json(res, 200, { ok: true });
        return true;
      }
      this.json(res, 200, { conversation: { id: m[1], title: msgs[0]?.text ?? '', messages: msgs }, folder: this.sessionFolders.get(m[1]) ?? null });
      return true;
    }
    m = /^\/v1\/folders(?:\/([^/]+)(\/rename|\/move)?)?$/.exec(path);
    if (m) {
      if (!m[1]) {
        if (req.method === 'POST') {
          const f = { id: 'f-' + this.nextId++, name: body.name, parent: body.parent ?? null };
          this.folders.push(f);
          this.json(res, 201, f);
        } else {
          this.json(res, 200, { folders: this.folders });
        }
        return true;
      }
      const f = this.folders.find((x) => x.id === m![1]);
      if (!f) {
        notFound('folder');
        return true;
      }
      if (m[2] === '/rename') {
        f.name = body.name;
      } else if (m[2] === '/move') {
        f.parent = body.parent;
      } else {
        this.folders = this.folders.filter((x) => x !== f).map((x) => (x.parent === f.id ? { ...x, parent: f.parent } : x));
        for (const [s, id] of [...this.sessionFolders]) {
          if (id === f.id) {
            if (f.parent) {
              this.sessionFolders.set(s, f.parent);
            } else {
              this.sessionFolders.delete(s);
            }
          }
        }
      }
      this.json(res, 200, { ok: true });
      return true;
    }
    switch (path) {
      case '/v1/undo': {
        const d = this.lastDelete;
        if (!d || body.undo !== 'undo-1') {
          notFound('delete to undo');
          return true;
        }
        const msgs = this.chats.get(d.chat)!;
        for (const r of d.removed) {
          msgs.splice(r.at, 0, r.m);
        }
        this.lastDelete = undefined;
        this.json(res, 200, { ok: true });
        return true;
      }
      case '/v1/link/offer':
        if (this.pair.state === 'linked') {
          this.json(res, 400, { error: { kind: 'pairing', message: 'this device is already linked' } });
          return true;
        }
        this.pair = { state: 'offering', link: 'LINK-CODE-42', expires_ms: Date.now() + 600000 };
        if (this.offerScript === 'match') {
          setTimeout(() => {
            if (this.pair.state === 'offering') {
              this.pair = { state: 'confirm', sas: '913-205' };
            }
          }, 300);
        }
        this.json(res, 200, { link: 'LINK-CODE-42', expires_ms: this.pair.expires_ms });
        return true;
      case '/v1/link/offer/confirm':
        if (this.pair.state !== 'confirm') {
          this.json(res, 409, { error: { kind: 'pairing', message: 'nothing to confirm yet' } });
          return true;
        }
        this.pair = body.matches ? { state: 'waiting', sas: this.pair.sas } : { state: 'failed', message: 'the codes did not match' };
        if (body.matches) {
          setTimeout(() => (this.pair = { state: 'linked', user: 'u-1', device: 'Test Bragi' }), 300);
        }
        this.json(res, 200, { ok: true, pair: this.pair });
        return true;
      case '/v1/link/offer/cancel':
        this.pair = { state: 'unpaired' };
        this.json(res, 200, { ok: true, pair: this.pair });
        return true;
    }
    return false;
  }

  private handle(req: http.IncomingMessage, res: http.ServerResponse, raw: string): void {
    const path = (req.url ?? '').split('?')[0];
    const body = raw ? JSON.parse(raw) : undefined;
    this.seen.push({ method: req.method ?? '', path, auth: req.headers.authorization, body });
    if (path === '/v1/health') {
      return this.json(res, 200, { ok: true, version: 'test' });
    }
    // The OpenAI-compatible stand-in (the direct target) takes any key.
    if (path === '/openai/v1/models') {
      return this.json(res, 200, { object: 'list', data: [{ id: 'direct-model' }] });
    }
    if (path === '/openai/v1/chat/completions') {
      if (body.stream) {
        res.writeHead(200, { 'content-type': 'text/event-stream' });
        for (const piece of ['Hello ', 'from ', 'direct']) {
          res.write(`data: ${JSON.stringify({ choices: [{ delta: { content: piece } }] })}\n\n`);
        }
        res.end('data: [DONE]\n\n');
        return;
      }
      const user: string = body.messages[body.messages.length - 1].content;
      const content = user.includes('<code_before_cursor>') ? this.suggestion : '```\n' + this.rewrite('    a - b') + '\n```';
      return this.json(res, 200, { choices: [{ message: { role: 'assistant', content } }] });
    }
    if (req.headers.authorization !== `Bearer ${TOKEN}`) {
      return this.json(res, 401, { error: { kind: 'unauthorized', message: 'a valid Bragi token is needed (bragi serve token)' } });
    }
    const ep = /^\/v1\/endpoints\/([^/]+)(\/test)?$/.exec(path);
    if (ep) {
      const e = this.endpoints.find((x) => x.id === ep[1]);
      if (!e) {
        return this.json(res, 404, { error: { kind: 'not_found', message: 'no such endpoint' } });
      }
      if (ep[2]) {
        return this.json(res, 200, { ok: true, models: ['m1', 'm2'], latency_ms: 4, message: 'ok' });
      }
      if (req.method === 'DELETE') {
        this.endpoints = this.endpoints.filter((x) => x !== e);
        return this.json(res, 200, { rebound: 0, unbound: 0 });
      }
      Object.assign(e, { name: body.name, base_url: body.base_url, model: body.model ?? null, mode: body.mode, enabled: body.enabled, has_key: body.clear_key ? false : body.api_key ? true : e.has_key });
      return this.json(res, 200, e);
    }
    if (this.chatRoutes(req, res, path, body)) {
      return;
    }
    const ms = /^\/v1\/mcp-servers\/([^/]+)(\/enabled|\/always-allow|\/test)?$/.exec(path);
    if (ms && this.mcpClient) {
      const s = this.mcpServers.find((x) => x.id === ms[1]);
      if (!s) {
        return this.json(res, 404, { error: { kind: 'not_found', message: 'no such server' } });
      }
      switch (ms[2]) {
        case '/enabled':
          s.enabled = body.enabled;
          return this.json(res, 200, s);
        case '/always-allow':
          s.always_allow = body.allow ? [...new Set([...s.always_allow, body.tool])] : s.always_allow.filter((t: string) => t !== body.tool);
          return this.json(res, 200, s);
        case '/test':
          return this.json(res, 200, { ok: true, tools: ['docs.search'], message: 'ok' });
      }
      if (req.method === 'PUT') {
        Object.assign(s, this.mcpView(body, s));
        return this.json(res, 200, s);
      }
      this.mcpServers = this.mcpServers.filter((x) => x !== s);
      return this.json(res, 200, { ok: true });
    }
    switch (path) {
      case '/v1/state':
        return this.json(res, 200, { linked: this.pair.state === 'linked', pair: this.pair, models: [{ id: 'gemma-4-e4b', status: 'warm' }, { id: 'qwen3-32b', status: 'loadable' }], default_model: 'gemma-4-e4b' });
      case '/v1/targets':
        return this.json(res, 200, {
          targets: [
            { label: 'Weirdnet pool', target: { kind: 'pool' }, models: ['gemma-4-e4b', 'qwen3-32b'] },
            { label: 'Ymir bridge · qwen3.6-35b-a3b', target: { kind: 'endpoint', id: 'e-1', model: 'qwen3.6-35b-a3b' }, models: [] },
          ],
        });
      case '/v1/code/suggest':
        return this.json(res, 200, { text: this.suggestion, model: body.model ?? 'gemma-4-e4b' });
      case '/v1/code/rewrite': {
        const ctx = body.context;
        const scope = body.scope === 'selection' && ctx.selection !== '' ? 'selection' : 'file';
        const target = scope === 'selection' ? ctx.selection : ctx.before + ctx.selection + ctx.after;
        return this.json(res, 200, { code: this.rewrite(target), scope, model: 'gemma-4-e4b' });
      }
      // bragi serve answers these with bragi-core's FileDiff (tested in crates/bragi-serve).
      case '/v1/code/hunks':
        return this.json(res, 200, computeHunks(body.old, body.new));
      case '/v1/code/apply':
        return this.json(res, 200, applyHunks(body.old, body.new, body.accepted));
      case '/v1/agent/approve': {
        this.approvals.push(body);
        const go = this.waitingCard;
        if (!go || body.request_id !== 'r-agent' || body.ask !== 1) {
          return this.json(res, 404, { error: { kind: 'not_found', message: "that card isn't waiting any more" } });
        }
        this.waitingCard = undefined;
        go(body.choice);
        return this.json(res, 200, { ok: true });
      }
      case '/v1/code/chat': {
        res.writeHead(200, { 'content-type': 'text/event-stream' });
        if (body.agent) {
          return this.agentChat(res, body);
        }
        const id = body.conversation ?? 'c-test';
        const name = body.video ? String(body.video.path).split(/[\\/]/).pop() : '';
        const video = body.video
          ? [
              {
                type: 'video',
                name,
                bubble: `video  ${name}  0:42  2 frames  no sound sent`,
                summary: `Video "${name}", 00:42, 2 frames sampled.`,
                duration_ms: 42000,
                sound: false,
                clip: body.video.path,
                frames: [
                  { at_ms: 10500, label: '[00:10.5]', mime: 'image/jpeg', data: '/9j/AA==' },
                  { at_ms: 31500, label: '[00:31.5]', mime: 'image/jpeg', data: '/9j/AQ==' },
                ],
              },
            ]
          : [];
        const events = [
          { type: 'conversation', id, request_id: 'r-1' },
          ...video,
          { type: 'started', node: 'node-1', model: 'gemma-4-e4b' },
          { type: 'delta', offset: 0, text: 'It adds ' },
          { type: 'delta', offset: 8, text: 'two numbers.' },
          { type: 'done', finish_reason: 'stop' },
        ];
        for (const e of events) {
          res.write(`data: ${JSON.stringify(e)}\n\n`);
        }
        this.record(body, id, 'It adds two numbers.');
        res.end();
        return;
      }
      case '/v1/links':
        return this.json(res, 200, { links: this.links });
      case '/v1/links/node':
        this.links.push({ address: body.address, peer: 'peer-' + this.nextId++, online: true });
        return this.json(res, 200, { message: 'added ' + body.address });
      case '/v1/links/remove': {
        const n = this.links.length;
        this.links = this.links.filter((l) => l.address !== body.address);
        return n === this.links.length ? this.json(res, 404, { error: { kind: 'not_found', message: "that link isn't in the list" } }) : this.json(res, 200, { ok: true });
      }
      case '/v1/link/accept':
        return this.json(res, 200, { sas: '123-456', device: 'Test node' });
      case '/v1/link/confirm':
        this.linked = body.matches;
        return this.json(res, 200, { ok: true, linked: body.matches });
      case '/v1/endpoints':
        if (req.method === 'POST') {
          const e = { id: 'e-' + this.nextId++, name: body.name, base_url: body.base_url, model: body.model ?? null, mode: body.mode, enabled: body.enabled, has_key: !!body.api_key, insecure: false };
          this.endpoints.push(e);
          return this.json(res, 201, e);
        }
        return this.json(res, 200, { endpoints: this.endpoints });
      case '/v1/endpoints/test':
        return this.json(res, 200, { ok: true, models: ['m'], latency_ms: 3, message: 'ok' });
      case '/v1/sessions':
        return this.json(res, 200, {
          sessions: [
            { id: 's-1', title: 'First chat', updated_ms: 1, messages: 2, target_label: 'Weirdnet pool', folder: this.sessionFolders.get('s-1') ?? null },
            ...[...this.chats].map(([id, msgs]) => ({ id, title: msgs[0]?.text ?? '', updated_ms: 2, messages: msgs.length, target_label: 'Weirdnet pool', folder: this.sessionFolders.get(id) ?? null })),
          ],
        });
      case '/v1/mcp-servers':
        if (!this.mcpClient) {
          return this.json(res, 404, { error: { kind: 'not_found', message: path } });
        }
        if (req.method === 'POST') {
          const s = { id: 'm-' + this.nextId++, ...this.mcpView(body) };
          this.mcpServers.push(s);
          return this.json(res, 201, s);
        }
        return this.json(res, 200, { servers: this.mcpServers });
      case '/v1/video/limits':
      case '/v1/video/limits/set': {
        if (path.endsWith('/set')) {
          if (/frames=/.test(body.limits)) {
            return this.json(res, 400, { error: { kind: 'bad_request', message: '`frames` isn\'t a key' } });
          }
          this.videoUser = body.limits;
        }
        const ready = /image_tokens=/.test(this.videoUser);
        const model = body.model ?? 'gemma-4-e4b';
        return this.json(res, 200, {
          model,
          reported: 'context=32768',
          user: this.videoUser,
          effective: ['context=32768', this.videoUser].filter(Boolean).join(' '),
          ready,
          chip: ready ? `video ready (context 32768 · yours: ${this.videoUser})` : `video needs image cost: set limits for ${model}`,
          keys: [{ key: 'image_tokens', about: 'tokens per frame' }],
          ...(path.endsWith('/set') ? { ignored: /context=/.test(body.limits) ? ['context'] : [] } : {}),
        });
      }
      case '/v1/cancel':
        return this.json(res, 200, { cancelled: true });
      default:
        return this.json(res, 404, { error: { kind: 'not_found', message: path } });
    }
  }
}
