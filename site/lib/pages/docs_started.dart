import 'package:jaspr/jaspr.dart';

import '../components/layout.dart';
import '../site.dart';

const _html = r'''
<p>Bragi is the chat app for your own <a href="${ymirSite}">Ymir</a> cloud. This guide covers getting the app, linking it to
your Weirdnet identity, and chatting. You need a running Ymir pool first: follow the
<a href="${ymirGuide}">Ymir deployment guide</a>, steps 1 and 2.</p>
<p><strong>What is released.</strong> Nothing in Bragi is released yet: no store listing and no signed downloads. You
build the apps from source. Bragi has so far run against a built-in demo inference node, not yet against a Ymir pool
on separate real machines.</p>

<h2 id="install">Install Bragi</h2>
<p>Build from source (see the <a href="${repo}#building">README</a> for the dependencies):</p>
<pre><code>git clone https://github.com/WeirdBrains/bragi && cd bragi
cargo run --release -p bragi-desktop             # the Linux desktop app (GPUI)
cargo run -p bragi-desktop -- --demo             # try it against a built-in demo node, no Ymir needed

cd app &amp;&amp; flutter build apk                      # the Android app (needs the Android SDK and NDK)</code></pre>
<p>The desktop app keeps its identity and history in a folder you can move with <code>--home DIR</code> or the
<code>BRAGI_HOME</code> environment variable. The planned formats and platforms are on the
<a href="${download}">Download</a> page.</p>

<h2 id="link">Link Bragi to your Weirdnet</h2>
<p>Bragi joins your Weirdnet identity like any of your devices, and it needs a device that is already linked (Ymir
or another Bragi) to admit it. Either side can show the code.</p>
<p><strong>Bragi shows the code:</strong></p>
<ol>
<li>Open Bragi. It says <em>Link Bragi to your Weirdnet</em>. Tap <strong>Show a link code</strong>.</li>
<li>Bragi shows a <code>weirdnet-bragi://link/…</code> code, as text and as a QR code. Press <strong>Copy code</strong> (it says <em>Copied</em> for a moment) to put it on the clipboard, on the desktop and on the phone. Bragi on the desktop also prints the code to its standard output when it shows it, so you can pick it up from a terminal too. The code works for two minutes.</li>
<li>On a Ymir device that is already in the pool, run <code>ymirctl pool add 'weirdnet-bragi://link/…'</code>. In another linked Bragi, open it as a code from another device.</li>
<li>Bragi shows <em>Do both screens show this code?</em> and six digits, and the other device shows the same six. Compare them. If they match, tap <strong>They match</strong>. If not, tap <strong>They don't</strong> and start again.</li>
<li>When the other device confirms too, Bragi goes online.</li>
</ol>
<p><strong>The other device shows the code:</strong> a linked Bragi can admit a new Ymir node. Run
<code>ymirctl pool link</code> on the node, open the <code>weirdnet-ymir://link/…</code> code in Bragi, compare the six
digits, and confirm on both. Bragi reads Ymir's codes as well as its own.</p>
<p>There is no QR scanner yet, so codes are pasted (Ctrl+V or Ctrl+Shift+V in the box for a code from another device). This linking is covered by automated tests and by an end-to-end test
of a headless Bragi against real Ymir nodes on one computer; it has not yet been tried across separate machines.</p>

<h2 id="chat">Chat</h2>
<ul>
<li>Bragi lists the chat models offered by every inference node in your pool, warm ones first. If the list is empty, check that a device in the pool runs the Ymir <code>inference</code> role (<code>ymirctl status</code>).</li>
<li>Pick a model and ask. The answer streams in token by token.</li>
<li><strong>Stop</strong> cancels the request and keeps what arrived; <strong>Continue</strong> picks the answer up again. A real model may rephrase a little at the seam.</li>
<li>History is saved on the device, one file per conversation. Syncing it between your devices is in progress.</li>
</ul>
<p>Phones are reachable only while the app is open, and Bragi needs the Ymir pool to be reachable over Weirdnet, either on
the same network or through a relay.</p>
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
            .replaceAll(r'${download}', siteLink('/download'))
            .replaceAll(r'${features}', siteLink('/features'))
            .replaceAll(r'${roadmap}', siteLink('/roadmap'))
            .replaceAll(r'${docs}', siteLink('/docs')),
      ),
    ]);
  }
}
