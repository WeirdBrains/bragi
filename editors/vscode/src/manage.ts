// The management side of `bragi serve`: the links, OpenAI-compatible endpoints, chats and MCP
// servers Bragi keeps, so the editor sets up what the apps set up. The data stays in Bragi's home;
// keys typed here go to Bragi once and come back only as "a key is set".

import type { BragiBackend } from './backends';
import { BragiError } from './types';

export interface LinkView {
  address: string;
  peer: string;
  online: boolean;
}

export type EndpointMode = 'fallback' | 'alongside';

export interface EndpointView {
  id: string;
  name: string;
  base_url: string;
  model?: string | null;
  mode: EndpointMode;
  enabled: boolean;
  has_key: boolean;
  insecure: boolean;
}

export interface EndpointInput {
  name: string;
  baseUrl: string;
  model?: string;
  mode?: EndpointMode;
  enabled?: boolean;
  /** Set the key. Sent once, never stored by the extension. */
  apiKey?: string;
  clearKey?: boolean;
}

export interface TestReport {
  ok: boolean;
  models: string[];
  latency_ms: number;
  message: string;
}

export interface SessionSummary {
  id: string;
  title: string;
  updated_ms: number;
  messages: number;
  target_label: string;
}

/**
 * An outside MCP server as Bragi lists it (`/v1/mcp-servers`, bragi-core's `McpServerView`): no
 * secrets, only which names have a value.
 */
export interface McpServerView {
  id: string;
  name: string;
  transport: 'stdio' | 'http';
  enabled: boolean;
  /** The command line or the address. */
  target: string;
  env_names: string[];
  header_names: string[];
  has_bearer: boolean;
  scope: { kind: 'all' | 'folder' | 'session'; value?: string };
  /** Tools (as offered, `<server>.<tool>`) that run without asking. */
  always_allow: string[];
  /** For a local command: it runs as you. */
  warning?: string | null;
}

export interface McpServerInput {
  name: string;
  transport: 'stdio' | 'http';
  enabled?: boolean;
  command?: string;
  args?: string[];
  cwd?: string;
  url?: string;
  /** Where its tools are offered: every chat, or code chats on `folder`. */
  scope?: 'all' | 'folder';
  folder?: string;
  /** Typed once, kept by Bragi's secret store. */
  secrets?: { env?: Record<string, string>; headers?: Record<string, string>; bearer?: string };
}

export class Manage {
  constructor(private readonly backend: () => BragiBackend) {}

  private api<T>(method: string, path: string, body?: unknown): Promise<T> {
    return this.backend().api<T>(method, path, body);
  }

  async links(): Promise<LinkView[]> {
    return (await this.api<{ links: LinkView[] }>('GET', '/v1/links')).links;
  }

  /** Reads a link code from another device; the returned code is compared on both screens. */
  linkAccept(code: string): Promise<{ sas: string; device: string }> {
    return this.api('POST', '/v1/link/accept', { code: code.trim() });
  }

  linkConfirm(matches: boolean): Promise<{ ok: boolean; linked: boolean }> {
    return this.api('POST', '/v1/link/confirm', { matches });
  }

  addNode(address: string): Promise<{ message: string }> {
    return this.api('POST', '/v1/links/node', { address: address.trim() });
  }

  removeLink(address: string): Promise<unknown> {
    return this.api('POST', '/v1/links/remove', { address });
  }

  async endpoints(): Promise<EndpointView[]> {
    return (await this.api<{ endpoints: EndpointView[] }>('GET', '/v1/endpoints')).endpoints;
  }

  private endpointBody(i: EndpointInput): Record<string, unknown> {
    return { name: i.name, base_url: i.baseUrl, model: i.model || undefined, mode: i.mode ?? 'alongside', enabled: i.enabled ?? true, api_key: i.apiKey || undefined, clear_key: i.clearKey || undefined };
  }

  addEndpoint(i: EndpointInput): Promise<EndpointView> {
    return this.api('POST', '/v1/endpoints', this.endpointBody(i));
  }

  editEndpoint(id: string, i: EndpointInput): Promise<EndpointView> {
    return this.api('PUT', `/v1/endpoints/${encodeURIComponent(id)}`, this.endpointBody(i));
  }

  removeEndpoint(id: string): Promise<unknown> {
    return this.api('DELETE', `/v1/endpoints/${encodeURIComponent(id)}`);
  }

  testEndpoint(id: string): Promise<TestReport> {
    return this.api('POST', `/v1/endpoints/${encodeURIComponent(id)}/test`);
  }

  testAddress(baseUrl: string, apiKey?: string): Promise<TestReport> {
    return this.api('POST', '/v1/endpoints/test', { base_url: baseUrl, api_key: apiKey || undefined });
  }

  async sessions(limit = 50): Promise<SessionSummary[]> {
    return (await this.api<{ sessions: SessionSummary[] }>('GET', `/v1/sessions?limit=${limit}`)).sessions;
  }

  deleteSession(id: string): Promise<unknown> {
    return this.api('DELETE', `/v1/sessions/${encodeURIComponent(id)}`);
  }

  // Outside MCP servers (`/v1/mcp-servers`). An older `bragi serve` without the routes answers
  // 404; these then fail with 'mcp_unavailable' and the UI says so.
  private async mcp<T>(method: string, path: string, body?: unknown): Promise<T> {
    try {
      return await this.api<T>(method, path, body);
    } catch (e) {
      if (e instanceof BragiError && e.status === 404) {
        throw new BragiError("This Bragi doesn't manage outside MCP servers yet (it needs the MCP client release). Update Bragi, then try again.", 'mcp_unavailable', 404);
      }
      throw e;
    }
  }

  async mcpServers(): Promise<McpServerView[]> {
    return (await this.mcp<{ servers: McpServerView[] }>('GET', '/v1/mcp-servers')).servers;
  }

  addMcpServer(i: McpServerInput): Promise<McpServerView> {
    return this.mcp('POST', '/v1/mcp-servers', i);
  }

  removeMcpServer(id: string): Promise<unknown> {
    return this.mcp('DELETE', `/v1/mcp-servers/${encodeURIComponent(id)}`);
  }

  setMcpServerEnabled(id: string, enabled: boolean): Promise<McpServerView> {
    return this.mcp('POST', `/v1/mcp-servers/${encodeURIComponent(id)}/enabled`, { enabled });
  }

  testMcpServer(id: string): Promise<{ ok: boolean; tools: string[]; message: string }> {
    return this.mcp('POST', `/v1/mcp-servers/${encodeURIComponent(id)}/test`);
  }

  /** Replaces a server's settings; without `secrets` the stored ones stay. */
  editMcpServer(id: string, i: McpServerInput): Promise<McpServerView> {
    return this.mcp('PUT', `/v1/mcp-servers/${encodeURIComponent(id)}`, i);
  }

  /** Gives or takes back "always allow" for one of the server's tools. */
  setMcpToolAlwaysAllowed(id: string, tool: string, allow: boolean): Promise<McpServerView> {
    return this.mcp('POST', `/v1/mcp-servers/${encodeURIComponent(id)}/always-allow`, { tool, allow });
  }
}
