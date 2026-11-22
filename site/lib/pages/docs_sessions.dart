import 'package:jaspr/jaspr.dart';

import '../components/layout.dart';
import '../site.dart';

const _html = r'''
<p>A <strong>session</strong> is a place to chat that sends to one target: your Weirdnet inference nodes, or one
OpenAI-compatible endpoint you add. Each session keeps its own model and its own history, and sessions sit in folders
you arrange. An <strong>endpoint</strong> can also be the fallback for the Weirdnet session: when none of your
inference nodes answers, Bragi sends to a Ymir bridge instead.</p>

<h2 id="sessions">Sessions</h2>
<p>The <strong>Sessions</strong> section sits at the top of the sidebar on the desktop and the drawer on the phone.
Every device starts with one session, <em>Weirdnet</em>, which sends to your inference nodes and holds the history you
had before sessions. It can be renamed and moved, not deleted.</p>
<ul>
<li><strong>New session:</strong> press <strong>+ Session</strong>, give it a name, and choose what it sends to:
Weirdnet or one of your endpoints. On the phone you can also pick its model; on the desktop it starts on the target's
first model and remembers the one you pick.</li>
<li><strong>Open:</strong> click or tap a session. The history list shows only its conversations, the model chips show
only its target's models, and the line under them says where the next message goes.</li>
<li><strong>Rename, change target, delete:</strong> on the desktop, pick the session and use the buttons under the
tree; on the phone, use the row's menu. Deleting a session keeps its conversations: they move to the Weirdnet session.</li>
<li>A session sends only to its target. A session on an endpoint never falls back to Weirdnet, and the Weirdnet session
only uses an endpoint when it is set as the fallback (below). Agent tasks run on your Weirdnet nodes, so they are
available in the Weirdnet session only.</li>
</ul>

<h2 id="folders">Folders</h2>
<ul>
<li><strong>New folder:</strong> press <strong>+ Folder</strong> (on the desktop it goes inside the picked folder; on
the phone use <em>New folder here</em> in a folder's menu). Folders can hold sessions and other folders.</li>
<li><strong>Fold:</strong> click or tap a folder to collapse or expand it.</li>
<li><strong>Move:</strong> drag a session or folder onto a folder, or onto the SESSIONS header for the top level (on the
phone, press and hold to drag). Or use <strong>Move</strong> (desktop) or <em>Move to…</em> (phone) and pick where it goes.
A folder can't go inside itself.</li>
<li><strong>Delete a folder:</strong> Bragi asks first. The sessions and folders in it move up to the folder it was in;
nothing is deleted with it.</li>
</ul>
<p>Sessions and folders are kept on the device, in <code>sessions.json</code> in Bragi's data folder. Each conversation
records its session, so its history syncs to your other devices as before; a device that doesn't have that session
shows the conversation under its Weirdnet session.</p>

<h2 id="endpoints">Endpoints</h2>
<p>Open <strong>Endpoints and links</strong> under the sessions. To add one, type a name, its URL and its token, and
press <strong>Check and add</strong>.</p>
<ul>
<li>The URL is the endpoint's OpenAI-compatible base, ending in <code>/v1</code> (Bragi adds it if you leave it out),
for example <code>https://ymir.example.ts.net:9443/v1</code>. It must be <code>https</code>, except on
<code>127.0.0.1</code> or <code>localhost</code>.</li>
<li>Bragi checks the token against the endpoint's <code>/v1/models</code> before it saves anything. A wrong token is
refused. The list also tells Bragi what it is talking to: a <strong>Ymir bridge</strong> marks each model with
<code>"owned_by": "weirdnet"</code> and a <code>weirdnet</code> object; anything else is shown as
<strong>OpenAI-compatible</strong>.</li>
<li>The token is kept in a private file in Bragi's data folder (<code>secrets/</code>, readable only by you), never
in the settings file, never in a log, and never shown again. Bragi has no system keychain support yet.</li>
<li>Certificates are checked against the system trust store, so an endpoint behind Tailscale certificates, or behind
a reverse proxy with your own certificate authority installed on the device, works without extra settings.</li>
<li>Bragi checks every endpoint every 30 seconds and shows whether it answers, how many models it lists, its default
model, and how many sessions send to it.</li>
</ul>
<p><strong>Test connection</strong> checks a URL and token against <code>/v1/models</code> and says what answered, without
saving anything.</p>

<h3 id="edit-endpoint">Edit or delete an endpoint</h3>
<ul>
<li><strong>Edit</strong> opens the endpoint's settings: its name, URL, token, default model and whether it is the
fallback. The token box starts empty: the saved token is never shown. Leave it empty to keep it, or type a new one
to replace it. A changed URL or token is tested before it is saved, and nothing changes if the test fails.</li>
<li>The <strong>default model</strong> is what a session on this endpoint asks for until you pick another in that
session (and what the Weirdnet session uses when it falls back to it). <em>The first one it lists</em> follows the
endpoint's own order.</li>
<li><strong>Delete</strong> asks first. If sessions send to the endpoint, Bragi says how many and where they should go:
Weirdnet, another endpoint, or nowhere for now (they say so until you pick a target). Their conversations are kept
either way, and the endpoint's saved token is deleted with it.</li>
</ul>

<h2 id="links">Weirdnet links</h2>
<p>The same screen lists your <strong>Weirdnet links</strong>: the node addresses this device dials, the ones link
codes brought and the ones you pasted into <strong>Link a device</strong>, each with a name and whether it is connected
now.</p>
<ul>
<li><strong>Add</strong> a name and an address (<code>/ip4/…/tcp/4848/p2p/…</code>, from <code>ymirctl pool
addresses</code>). Bragi dials it first and adds it only if that node answers.</li>
<li><strong>Edit</strong> renames a link or changes its address; a new address is tested the same way, and nothing
changes if the test fails.</li>
<li><strong>Delete</strong> asks first, then forgets the address and stops dialing it. No session is bound to one link
(sessions use the whole pool), so no session changes and no conversation is deleted.</li>
<li>Addresses that come from Bragi's own configuration can be renamed but not edited or deleted.</li>
</ul>
<p>Deleting a link does not unlink this device from your Weirdnet; that is done from another linked device.</p>

<h2 id="fallback">The Ymir bridge fallback</h2>
<p>A <a href="${ymirSite}">Ymir</a> node can run the OpenAI bridge, which offers your pool's models at an
OpenAI-compatible <code>/v1</code> behind a bearer token. Add it as an endpoint and switch on <strong>Use as
fallback</strong> (desktop) or <strong>Fallback for the Weirdnet session</strong> (phone). Only a Ymir bridge can be
the fallback.</p>
<ul>
<li>Your Weirdnet nodes always come first. The Weirdnet session sends to the bridge only when no inference node is
reachable.</li>
<li>If an answer breaks off partway on Weirdnet (the node goes away and no other node can resume it), Bragi continues
the same answer on the bridge from where it stopped, and says so above the message box.</li>
<li>The line under the model chips always says where the next message goes: <em>Your Weirdnet · 2 inference nodes,
ymir as fallback</em>, <em>No inference node reachable · using the ymir Ymir bridge</em>, or <em>Sends to … only</em>
for an endpoint session. Each answer from an endpoint is marked <em>via</em> its name.</li>
</ul>
<p>To set it up: on the Ymir node that runs the bridge, note its URL (reached over your tailnet, for example
<code>https://ymir.example.ts.net:9443/v1</code>) and its token. In Bragi, open <strong>Endpoints and links</strong>, add
the URL and paste the token, check that it shows as a Ymir bridge, and turn on the fallback.</p>
<p>More: <a href="${started}">Getting started</a>, <a href="${features}">Features</a>, and the
<a href="${docs}">documentation index</a>.</p>
''';

class SessionsPage extends StatelessComponent {
  const SessionsPage({super.key});

  @override
  Component build(BuildContext context) {
    return Component.fragment([
      pageHero(
        eyebrow: 'Docs · Guides',
        title: 'Sessions, endpoints, links and the fallback',
        lede: 'Send each session to Weirdnet or one API, keep them in folders, and fall back to a Ymir bridge.',
      ),
      prose(
        _html
            .replaceAll(r'${ymirSite}', ymirSiteUrl)
            .replaceAll(r'${started}', siteLink('/docs/guides/getting-started'))
            .replaceAll(r'${features}', siteLink('/features'))
            .replaceAll(r'${docs}', siteLink('/docs')),
      ),
    ]);
  }
}
