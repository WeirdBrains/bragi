import 'package:jaspr/jaspr.dart';

import '../components/layout.dart';
import '../site.dart';

const _html = r'''
<p><code>bragi mcp</code> runs Bragi as a <a href="https://modelcontextprotocol.io">Model Context Protocol</a> server,
so tools such as Cursor, Claude Desktop and VS Code can ask your own models through Bragi, read and file your
sessions, get code suggestions and rewrites as diffs, and see your endpoints and models. It uses the same Bragi folder as the app: chats it makes show up in the
app's history, and it sends to the same Weirdnet pool and endpoints. Link Bragi first (see
<a href="${started}#link">Getting started</a>).</p>

<h2 id="run">Run it</h2>
<pre><code>bragi mcp --root ~/projects/app           # stdio: the client starts it (the usual way)
bragi mcp --http                          # streamable HTTP on http://127.0.0.1:4878/mcp
bragi mcp token                           # the bearer token for --http
bragi mcp status                          # linked or not, the nodes, models and endpoints
bragi mcp --demo                          # try it against a built-in demo node, no Ymir needed</code></pre>
<p>The desktop app's <code>bragi</code> program runs it as <code>bragi mcp</code>; there is also a separate
<code>bragi-mcp</code> program (<code>cargo build --release -p bragi-mcp</code>) for machines without the desktop app.
Both take the same options.</p>
<ul>
<li><code>--root DIR</code>: the project folder. Files you ask Bragi to attach to a chat, or to save from one, must be inside it. Default: the folder the client starts it in.</li>
<li><code>--http [ADDR]</code>: serve HTTP instead of stdio, on a loopback address only (default <code>127.0.0.1:4878</code>). Every request needs <code>Authorization: Bearer TOKEN</code>; requests from web pages (an <code>Origin</code> header) and for other host names are refused.</li>
<li><code>--allow-writes</code>: also offer the tools that write into the project folder: applying a code change (and undoing it), and saving a chat's file. Off by default.</li>
<li><code>--allow-deletes</code>: also offer the tools that delete a session or a folder. Off by default.</li>
<li><code>--home DIR</code>: another Bragi folder (default <code>$BRAGI_HOME</code>, else <code>~/.local/share/bragi</code>).</li>
<li><code>--timeout SECS</code>: stop an answer that takes longer (default 600).</li>
</ul>

<h2 id="cursor">Cursor</h2>
<p>In <code>~/.cursor/mcp.json</code> (every project) or <code>.cursor/mcp.json</code> (one project):</p>
<pre><code>{
  "mcpServers": {
    "bragi": {
      "command": "bragi",
      "args": ["mcp", "--root", "${workspaceFolder}"]
    }
  }
}</code></pre>

<h2 id="claude">Claude Desktop</h2>
<p>In <code>claude_desktop_config.json</code> (Settings, Developer, Edit Config). Claude Desktop does not search your
<code>PATH</code>, so give the full path to the program and a project folder:</p>
<pre><code>{
  "mcpServers": {
    "bragi": {
      "command": "/usr/bin/bragi",
      "args": ["mcp", "--root", "/home/you/projects/app"]
    }
  }
}</code></pre>

<h2 id="vscode">VS Code</h2>
<p>In <code>.vscode/mcp.json</code>:</p>
<pre><code>{
  "servers": {
    "bragi": {
      "type": "stdio",
      "command": "bragi",
      "args": ["mcp", "--root", "${workspaceFolder}"]
    }
  }
}</code></pre>

<h2 id="http">Over HTTP</h2>
<p>Start <code>bragi mcp --http</code> yourself and point the client at <code>http://127.0.0.1:4878/mcp</code> with the
token from <code>bragi mcp token</code>. In VS Code, let it ask for the token once and keep it in its secret storage:</p>
<pre><code>{
  "inputs": [{ "type": "promptString", "id": "bragi-token", "description": "bragi mcp token", "password": true }],
  "servers": {
    "bragi": {
      "type": "http",
      "url": "http://127.0.0.1:4878/mcp",
      "headers": { "Authorization": "Bearer ${input:bragi-token}" }
    }
  }
}</code></pre>
<p>The token is the same one <code>bragi serve</code> uses for editor extensions, kept in the Bragi folder (readable
only by you). Delete the file it names (<code>bragi mcp token-path</code>) to make a new one.</p>

<h2 id="tools">Tools</h2>
<ul>
<li><code>bragi_chat</code>: ask a model and get the whole answer. Continues a session or starts one (optionally in a folder), sends to the pool or an endpoint, and can attach text files, images (PNG, JPEG, GIF, WebP) and audio (WAV, MP3) from the project folder.</li>
<li><code>bragi_sessions_list</code>, <code>bragi_session_get</code>: sessions newest first (all, or one folder's), and one session's messages.</li>
<li><code>bragi_folders_list</code>, <code>bragi_folder_create</code>, <code>bragi_folder_move</code>, <code>bragi_session_move</code>: file sessions in folders.</li>
<li><code>bragi_endpoints_list</code>, <code>bragi_models_list</code>: where a chat can go, and the models of the pool or of one endpoint. API keys are never shown.</li>
<li><code>bragi_files_list</code>, <code>bragi_file_fetch</code>: the files in a session, both attached ones and the files its answers wrote (code blocks that name a file); images and sounds come back as MCP image and audio content.</li>
<li><code>bragi_code_suggest</code>, <code>bragi_code_rewrite</code>: ask a model to review files and folders (respecting <code>.gitignore</code>, never reading secrets), or to rewrite a file or a line range (<code>src/lib.rs:10-40</code>). They return a unified diff, the model's explanation and a proposal id, write nothing, and stay out of chat history. It is the same code the terminal app and the editor extension use.</li>
<li><code>bragi_status</code>: linked or not, the nodes, and what to do when nothing answers.</li>
<li>With <code>--allow-writes</code>: <code>bragi_code_apply</code> writes a proposal, or the hunks you pick, and refuses when a file changed since; <code>bragi_code_undo</code> puts the files back (the undo journal is in the Bragi folder, not the project). <code>bragi_file_save</code> saves one of those files into the project folder; it adds <code> (2)</code> to the name rather than replace a file, unless asked to overwrite.</li>
<li>With <code>--allow-deletes</code>: <code>bragi_session_delete</code> (on every linked device) and <code>bragi_folder_delete</code> (what the folder holds moves up a level).</li>
</ul>
<h2 id="prompts">Prompts</h2>
<p>Prompts are ready-made requests your client fills in and sends to its own model (in Cursor and VS Code, type
<code>/</code> in the chat; in Claude Desktop, the + menu). Files must be inside the project folder; secrets
(<code>.env</code>, keys), binaries and big files are refused, as with the code tools.</p>
<ul>
<li><code>bragi_review_file</code> (<code>path</code>, optional <code>focus</code>): review a file, or lines of it (<code>src/lib.rs:10-40</code>).</li>
<li><code>bragi_rewrite</code> (<code>path</code>, <code>instruction</code>): the same request Bragi's own rewrite sends, answered as <code>bragi-edit</code> blocks.</li>
<li><code>bragi_explain</code> (<code>path</code> or <code>selection</code>, optional <code>language</code>): explain a file, some lines, or pasted code.</li>
<li><code>bragi_session_summary</code> (<code>session</code>): summarize a Bragi session.</li>
<li><code>bragi_continue_session</code> (<code>session</code>, optional <code>prompt</code>): a session's last 40 messages as the conversation so far, then your next message.</li>
</ul>
<p><strong>Resources:</strong> <code>bragi://status</code>, <code>bragi://sessions</code>, each session as Markdown at
<code>bragi://sessions/{id}</code>, and its files at <code>bragi://sessions/{id}/files/{file}</code>.</p>
<p>Bragi is a <a href="https://weirdbrains.com">Weird Brains</a> project. More: <a href="${docs}">documentation index</a>.</p>
''';

class McpGuidePage extends StatelessComponent {
  const McpGuidePage({super.key});

  @override
  Component build(BuildContext context) {
    return Component.fragment([
      pageHero(
        eyebrow: 'Docs · Guides',
        title: 'MCP server',
        lede: 'Use Bragi from Cursor, Claude Desktop, VS Code and other MCP clients.',
      ),
      prose(_html.replaceAll(r'${started}', siteLink('/docs/guides/getting-started')).replaceAll(r'${docs}', siteLink('/docs'))),
    ]);
  }
}
