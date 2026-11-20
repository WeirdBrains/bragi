// A stand-in for `bragi serve` and for an OpenAI-compatible server, on 127.0.0.1.

import * as http from 'http';
import { AddressInfo } from 'net';

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
    switch (path) {
      case '/v1/state':
        return this.json(res, 200, { linked: true, models: [{ id: 'gemma-4-e4b', status: 'warm' }, { id: 'qwen3-32b', status: 'loadable' }], default_model: 'gemma-4-e4b' });
      case '/v1/code/suggest':
        return this.json(res, 200, { text: this.suggestion, model: body.model ?? 'gemma-4-e4b' });
      case '/v1/code/rewrite': {
        const ctx = body.context;
        const scope = body.scope === 'selection' && ctx.selection !== '' ? 'selection' : 'file';
        const target = scope === 'selection' ? ctx.selection : ctx.before + ctx.selection + ctx.after;
        return this.json(res, 200, { code: this.rewrite(target), scope, model: 'gemma-4-e4b' });
      }
      case '/v1/code/chat': {
        res.writeHead(200, { 'content-type': 'text/event-stream' });
        const id = body.conversation ?? 'c-test';
        const events = [
          { type: 'conversation', id, request_id: 'r-1' },
          { type: 'started', node: 'node-1', model: 'gemma-4-e4b' },
          { type: 'delta', offset: 0, text: 'It adds ' },
          { type: 'delta', offset: 8, text: 'two numbers.' },
          { type: 'done', finish_reason: 'stop' },
        ];
        for (const e of events) {
          res.write(`data: ${JSON.stringify(e)}\n\n`);
        }
        res.end();
        return;
      }
      case '/v1/cancel':
        return this.json(res, 200, { cancelled: true });
      default:
        return this.json(res, 404, { error: { kind: 'not_found', message: path } });
    }
  }
}
