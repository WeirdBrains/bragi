// The Bragi activity-bar view "Connections": what Bragi is linked to and set up with, with the
// commands to change it. Everything shown is read from `bragi serve`; nothing is kept here.

import * as vscode from 'vscode';
import { Manage } from './manage';
import type { ServeManager } from './serve';
import { describeServe } from './serve';
import { BragiError } from './types';

export class Node extends vscode.TreeItem {
  constructor(
    label: string,
    readonly kind: 'group' | 'server' | 'link' | 'endpoint' | 'mcpServer' | 'mcpSelf' | 'note',
    state: vscode.TreeItemCollapsibleState = vscode.TreeItemCollapsibleState.None,
    readonly key?: string,
  ) {
    super(label, state);
    this.contextValue = kind;
    if (key) {
      this.id = `${kind}:${key}`;
    }
  }
}

const GROUPS = ['Local server', 'Links', 'OpenAI connections', 'MCP servers (outside)', 'Bragi as an MCP server'] as const;

export class ConnectionsView implements vscode.TreeDataProvider<Node> {
  static readonly id = 'bragi.connections';
  private readonly changed = new vscode.EventEmitter<Node | undefined>();
  readonly onDidChangeTreeData = this.changed.event;

  constructor(
    private readonly manage: Manage,
    private readonly serve: ServeManager,
    private readonly mcpRunning: () => boolean,
  ) {}

  refresh(): void {
    this.changed.fire(undefined);
  }

  getTreeItem(n: Node): vscode.TreeItem {
    return n;
  }

  async getChildren(parent?: Node): Promise<Node[]> {
    if (!parent) {
      return GROUPS.map((g) => {
        const n = new Node(g, 'group', vscode.TreeItemCollapsibleState.Expanded);
        return n;
      });
    }
    if (parent.kind !== 'group') {
      return [];
    }
    try {
      switch (parent.label) {
        case GROUPS[0]:
          return [this.serverNode()];
        case GROUPS[1]:
          return await this.links();
        case GROUPS[2]:
          return await this.endpoints();
        case GROUPS[3]:
          return await this.mcpServers();
        default:
          return [this.mcpSelf()];
      }
    } catch (e) {
      return [this.problem(e)];
    }
  }

  private serverNode(): Node {
    const d = describeServe(this.serve.state, this.serve.port);
    const n = new Node(d.text.replace(/\$\([^)]*\)\s*/g, '') || 'Bragi', 'server');
    n.description = this.serve.state.kind;
    n.tooltip = d.tooltip;
    n.iconPath = new vscode.ThemeIcon(this.serve.state.kind === 'running' ? 'server' : 'debug-disconnect');
    return n;
  }

  private mcpSelf(): Node {
    const running = this.mcpRunning();
    const n = new Node(running ? 'Running in a terminal' : 'Not running', 'mcpSelf');
    n.description = running ? 'bragi mcp' : '';
    n.tooltip = 'Bragi\'s own MCP server for other tools. "Copy MCP Client Config" gives the lines Cursor, VS Code or Claude Desktop need.';
    n.iconPath = new vscode.ThemeIcon(running ? 'play-circle' : 'circle-outline');
    return n;
  }

  private async links(): Promise<Node[]> {
    const links = await this.manage.links();
    if (links.length === 0) {
      return [this.note('No links yet. Use "Bragi: Link to an Inference Node".')];
    }
    return links.map((l) => {
      const n = new Node(l.peer ? `${l.peer.slice(0, 12)}…` : l.address, 'link', vscode.TreeItemCollapsibleState.None, l.address);
      n.description = l.online ? 'online' : 'offline';
      n.tooltip = l.address;
      n.iconPath = new vscode.ThemeIcon(l.online ? 'vm-active' : 'vm-outline');
      return n;
    });
  }

  private async endpoints(): Promise<Node[]> {
    const all = await this.manage.endpoints();
    if (all.length === 0) {
      return [this.note('No OpenAI connections yet. Use "Bragi: Add OpenAI Connection".')];
    }
    return all.map((e) => {
      const n = new Node(e.name, 'endpoint', vscode.TreeItemCollapsibleState.None, e.id);
      n.description = [e.mode === 'alongside' ? 'alongside' : 'fallback', e.has_key ? 'key set' : 'no key', e.enabled ? '' : 'off'].filter(Boolean).join(' · ');
      n.tooltip = `${e.base_url}${e.insecure ? '\nPlain http to another machine: the key and chat can be read on the way.' : ''}`;
      n.iconPath = new vscode.ThemeIcon(e.insecure ? 'warning' : 'plug');
      return n;
    });
  }

  private async mcpServers(): Promise<Node[]> {
    try {
      const all = await this.manage.mcpServers();
      if (all.length === 0) {
        return [this.note('No outside MCP servers. Use "Bragi: Add MCP Server".')];
      }
      return all.map((s) => {
        const n = new Node(s.name, 'mcpServer', vscode.TreeItemCollapsibleState.None, s.id);
        const always = s.always_allow?.length ? ` · ${s.always_allow.length} always allowed` : '';
        const scope = s.scope?.kind === 'folder' ? ` · ${s.scope.value}` : '';
        n.description = `${s.transport}${s.enabled ? '' : ' · off'}${scope}${always}`;
        n.tooltip = [s.target, s.warning, s.always_allow?.length ? `Always allowed: ${s.always_allow.join(', ')}` : ''].filter(Boolean).join('\n');
        n.iconPath = new vscode.ThemeIcon(s.enabled ? 'tools' : 'circle-slash');
        return n;
      });
    } catch (e) {
      if (e instanceof BragiError && e.kind === 'mcp_unavailable') {
        return [this.note("This Bragi can't manage outside MCP servers yet (needs the MCP client release).")];
      }
      throw e;
    }
  }

  private note(text: string): Node {
    const n = new Node(text, 'note');
    n.iconPath = new vscode.ThemeIcon('info');
    return n;
  }

  private problem(e: unknown): Node {
    if (e instanceof BragiError && e.kind === 'unauthorized') {
      const n = new Node('No API token yet. Click to enter one.', 'note');
      n.command = { command: 'bragi.setToken', title: 'Set Bragi API Token' };
      n.iconPath = new vscode.ThemeIcon('key');
      return n;
    }
    const n = new Node(e instanceof Error ? e.message : String(e), 'note');
    n.iconPath = new vscode.ThemeIcon('warning');
    return n;
  }
}
