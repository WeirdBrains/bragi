import 'package:jaspr/jaspr.dart';

import '../components/layout.dart';
import '../site.dart';

const _html = r'''
<p>Bragi is the chat app for your own <a href="${ymirSite}">Ymir</a> cloud. This guide covers getting the app, linking it to
your Weirdnet identity, and chatting. You need a running Ymir pool first: follow the
<a href="${ymirGuide}">Ymir deployment guide</a>, steps 1 and 2.</p>
<p><strong>What is released.</strong> Bragi ${version} has downloads for Linux, macOS, Windows, Android and iOS on the
<a href="${download}">Download</a> page. The Android apks are signed with a debug key and the iOS ipa is unsigned, for
sideloading; Bragi is not on a store.</p>

<h2 id="install">Install Bragi</h2>
<p>Download the file for your platform from the <a href="${download}">Download</a> page: a deb, rpm, Arch package or
tarball on Linux, the dmg on macOS, the installer on Windows (for your user only, no admin needed), the apk on Android,
and the ipa on iOS (sideload it with AltStore, SideStore or Sideloadly). Or build from source (see the
<a href="${repo}#building">README</a>):</p>
<pre><code>git clone https://github.com/WeirdBrains/bragi &amp;&amp; cd bragi
cargo run --release -p bragi-desktop -- desktop  # the desktop app (GPUI)
cargo run -p bragi-desktop -- desktop --demo     # try it against a built-in demo node, no Ymir needed
cargo run -p bragi-desktop                       # `bragi` alone is the terminal app</code></pre>
<p>The desktop app keeps its identity and history in a folder you can move with <code>--home DIR</code> or the
<code>BRAGI_HOME</code> environment variable.</p>

<h2 id="link">Link Bragi to your Weirdnet</h2>
<p>Bragi joins your Weirdnet identity like any of your devices, and it needs a device that is already linked (Ymir
or another Bragi) to admit it. Either side can show the code.</p>
<p><strong>Bragi shows the code:</strong></p>
<ol>
<li>Open Bragi. It says <em>Link Bragi to your Weirdnet</em>. Tap <strong>Show a link code</strong>.</li>
<li>Bragi shows a <code>weirdnet-bragi://link/…</code> code, as text and as a QR code. Press <strong>Copy code</strong> (it says <em>Copied</em> for a moment) to put it on the clipboard, on the desktop and on the phone. Bragi on the desktop also prints the code to its standard output when it shows it, so you can pick it up from a terminal too. The code works for two minutes.</li>
<li>On a Ymir device that is already in the pool, run <code>ymirctl pool add 'weirdnet-bragi://link/…'</code>. In another linked Bragi, open it as a code from another device.</li>
<li>Bragi shows <em>Do both screens show this code?</em> and six digits, and the other device shows the same six. Compare them. If they match, tap <strong>They match</strong>. If not, tap <strong>They don't</strong> and start again.</li>
<li>When the other device confirms too, Bragi goes online. Ymir's <code>pool add</code> also tells Bragi where your nodes can be reached, and Bragi keeps those addresses.</li>
</ol>
<p><strong>Nodes on another network:</strong> Bragi finds Ymir nodes on its local network by itself. If yours are somewhere
it can't discover (another network, or nodes with discovery turned off) and Bragi says <em>No inference node reachable
yet</em>, run <code>ymirctl pool addresses</code> on a Ymir node and paste one of the addresses it prints
(<code>/ip4/…/tcp/4848/p2p/…</code>) into the <strong>Link a device</strong> box, then press <strong>Link</strong>. Bragi
dials it at once and remembers it.</p>
<p><strong>The other device shows the code:</strong> a linked Bragi can admit a new Ymir node. Run
<code>ymirctl pool link</code> on the node, open the <code>weirdnet-ymir://link/…</code> code in Bragi, compare the six
digits, and confirm on both. Bragi reads Ymir's codes as well as its own.</p>
<p><strong>A server without your identity seed:</strong> to link a Ymir server so it never holds your identity seed, run
<code>ymirctl pool link --cert-only</code> on it and, in the terminal app, <code>bragi-tui link 'weirdnet-ymir://link/…'
--server inference</code> (or <code>--server model</code> for a node that only runs models). The server gets a certificate
and a signed grant for that role; it serves and syncs as usual but cannot link or revoke devices, and revoking it later
never means a new identity. It needs a Ymir with cert-only links; an older one fails the link and keeps nothing.</p>
<p>An inference server linked this way reads your conversations with a key for them alone. Chats written before the link
move to that key on the next sync pass with every device reached, and back again once no such server is left. Every
device that shares chats with it needs Bragi 0.14.0 or newer (older ones can't open them).</p>
<p>Paste a code from another device into the box (Ctrl+V or Ctrl+Shift+V).</p>

<h2 id="chat">Chat</h2>
<ul>
<li>Bragi lists the chat models offered by every inference node in your pool, warm ones first. If the list is empty, check that a device in the pool runs the Ymir <code>inference</code> role (<code>ymirctl status</code>) and that Bragi can reach it (see <em>Nodes on another network</em> above).</li>
<li>Pick a model and ask. The answer streams in token by token.</li>
<li><strong>Stop</strong> cancels the request and keeps what arrived; <strong>Continue</strong> picks the answer up again. A real model may rephrase a little at the seam.</li>
<li>History is saved on the device, one file per conversation. It syncs between your linked devices.</li>
</ul>
<p>Phones are reachable only while the app is open, and Bragi needs the Ymir pool to be reachable over Weirdnet, either on
the same network or through a relay.</p>
<h2 id="terminal">The terminal app: open a folder or file</h2>
<p>Bragi also runs in a terminal (<code>bragi-tui</code>, over SSH too). Give it a path and it opens code mode there, with
the files as context and suggestions, rewrites and hunk review limited to that folder:</p>
<pre><code>bragi .                  # code mode on the current folder
bragi ~/src/app          # on another folder
bragi src/main.rs        # on one file, rooted at its git repository (or its folder)
bragi-tui --path serve   # a folder named like a command; ./serve works too</code></pre>
<p>With a folder, a <strong>file picker</strong> opens: a tree of the folder (<code>.gitignore</code> is respected;
<code>target/</code>, <code>node_modules/</code> and <code>.git/</code> are hidden) with file sizes and a marker on what is in
the context. Move with the arrows or <code>j</code> and <code>k</code>, press <code>Space</code> to add or remove a file (a folder adds
all of its files), <code>Enter</code> to open a folder or add a file and close the picker, <code>/</code> to filter by typing, and
<code>Esc</code> to close. The bottom line shows the context's size against its limit. Open it again from code mode with
<code>f</code>, <code>Ctrl+F</code> or <code>/files</code>. A file named on the command line goes straight into the context.</p>
<p>A relative path, <code>.</code>, <code>..</code> and <code>~</code> are resolved first, and a path that doesn't exist is
an error. <code>bragi status</code>, <code>bragi link</code> and the other commands work as before. <code>bragi PATH</code>
starts <code>bragi-tui</code>, so install it beside <code>bragi</code> (the packages do).</p>

<p>More: <a href="${features}">Features</a>, the <a href="${roadmap}">roadmap</a>, and the
<a href="${docs}">documentation index</a>. Bragi is a <a href="https://weirdbrains.com">Weird Brains</a> project.</p>
''';

class GettingStartedPage extends StatelessComponent {
  const GettingStartedPage({super.key});

  @override
  Component build(BuildContext context) {
    return Component.fragment([
      pageHero(
        eyebrow: 'Docs · Guides',
        title: 'Getting started',
        lede: 'Install Bragi, link it to your Weirdnet, and chat.',
      ),
      prose(
        _html
            .replaceAll(r'${ymirSite}', ymirSiteUrl)
            .replaceAll(r'${ymirGuide}', '${ymirSiteUrl}docs/guides/deployment')
            .replaceAll(r'${repo}', repoUrl)
            .replaceAll(r'${version}', latestVersion)
            .replaceAll(r'${download}', siteLink('/download'))
            .replaceAll(r'${features}', siteLink('/features'))
            .replaceAll(r'${roadmap}', siteLink('/roadmap'))
            .replaceAll(r'${docs}', siteLink('/docs')),
      ),
    ]);
  }
}
