import 'package:jaspr/dom.dart';
import 'package:jaspr/jaspr.dart';

import '../components/layout.dart';
import '../components/shots.dart';
import '../site.dart';

const _html = r'''
<h2 id="pairing">Pairing</h2>
<p>Bragi links to your Weirdnet identity like any of your devices. Either it shows a code for a device that is
already linked, or, as a linked device, it opens another device's code. Both screens show the same six digits to
compare, so a stranger can't slip in. Bragi's own codes use the <code>weirdnet-bragi://</code> scheme, and it also opens
codes from Ymir (<code>weirdnet-ymir://</code>), whose <code>ymirctl pool add</code> in turn opens Bragi's.
Paste the code from the other device into Bragi to link.</p>
<h2 id="models">Models</h2>
<p>Bragi lists the chat models your Ymir inference node offers, warm ones first, with live status (warm, busy, loads
on use). With several inference nodes, Bragi shows one combined list (B2).</p>
<h2 id="chat">Chat</h2>
<p>Answers stream in token by token. <strong>Stop</strong> cancels the request and keeps what arrived;
<strong>Continue</strong> picks the answer up again. A real model may
rephrase a little at the seam, because the continue request carries the partial answer plus an instruction. History is saved
locally, one file per conversation, and survives a restart. History syncs between your linked devices (B4).</p>
<h2 id="platforms">Desktop and phone</h2>
<p>The desktop app (GPUI: Linux, macOS and Windows) and the phone app (Flutter: Android and iOS) share one Rust core, so
they pair, list models, stream, stop and continue the same way. Downloads for every platform are on the
<a href="${download}">Download</a> page.</p>
''';

class FeaturesPage extends StatelessComponent {
  const FeaturesPage({super.key});

  @override
  Component build(BuildContext context) {
    return Component.fragment([
      pageHero(
        eyebrow: 'Features',
        title: 'Pair, pick a model, chat',
        lede: 'What Bragi does today, on both desktop and phone.',
      ),
      prose(_html.replaceAll(r'${download}', siteLink('/download'))),
      div(classes: 'wrap shots-wrap', [
        div(classes: 'shots', [
          shot('desktop-04-models.png', 'Bragi desktop model list', 'Models from your cloud, with status.'),
          shot(
            'desktop-07-stopped.png',
            'Bragi desktop with a stopped answer',
            'Stopped partway; Continue picks it up.',
          ),
        ]),
      ]),
    ]);
  }
}
