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
<p>Bragi joins your Weirdnet identity like any of your devices, and it needs another already-linked device to admit it.</p>
<ol>
<li>Open Bragi. It says <em>Link Bragi to your Weirdnet</em>. Tap <strong>Show a link code</strong>.</li>
<li>Bragi shows a <code>weirdnet-bragi://link/…</code> code, as text and as a QR code. On the phone you can copy it. The code works for two minutes. There is no QR scanner yet, so codes are pasted.</li>
<li>Open the code on a device that is already linked and accepts Bragi codes, such as another linked Bragi.</li>
<li>Bragi shows <em>Do both screens show this code?</em> and six digits. Compare them with the other device. If they match, tap <strong>They match</strong>. If not, tap <strong>They don't</strong>, and start again.</li>
<li>When the other device confirms too, Bragi goes online.</li>
</ol>
<p>A linked Bragi can also admit another device: it opens that device's code and shows the same six-digit check.
The Ymir command line, <code>ymirctl pool add</code>, reads <code>weirdnet-ymir://</code> and
<code>weirdnet-morph://</code> codes today, not <code>weirdnet-bragi://</code> ones. See
<a href="${ymirGuide}#bragi">the Ymir guide</a> for the current state.</p>

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
