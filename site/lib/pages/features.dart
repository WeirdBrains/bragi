import 'package:jaspr/dom.dart';
import 'package:jaspr/jaspr.dart';

import '../components/layout.dart';
import '../components/shots.dart';

const _html = '''
<h2 id="pairing">Pairing</h2>
<p>Bragi links to your Weirdnet identity like any of your devices. Either it shows a code for a device that is
already linked, or, as a linked device, it opens another device's code. Both screens show the same six digits to
compare, so a stranger can't slip in. Link codes use the <code>weirdnet-bragi://</code> scheme. For now they are pasted
in; a QR scanner is not built yet.</p>
<h2 id="models">Models</h2>
<p>Bragi lists the chat models your Ymir inference node offers, warm ones first, with live status (warm, busy, loads
on use). A multi-node catalog comes with the pool client (B2).</p>
<h2 id="chat">Chat</h2>
<p>Answers stream in token by token. <strong>Stop</strong> cancels the request and keeps what arrived;
<strong>Continue</strong> picks the answer up again. A continued answer is exact on the demo node, but a real model may
rephrase at the seam, because the continue request carries the partial answer plus an instruction. History is saved
locally, one file per conversation, and survives a restart. Synced history is planned (B4).</p>
<h2 id="platforms">Desktop and phone</h2>
<p>The desktop app (GPUI, Linux) and the phone app (Flutter, Android) share one Rust core, so they pair, list models,
stream, stop and continue the same way. A shell that lags a milestone is treated as a bug. iOS, macOS and Windows are
planned (B3).</p>
<h2 id="status">What is tested</h2>
<p>Both apps pass the same end-to-end scenario (pair, list models, chat, stop, continue) against a simulated
inference node. They have not yet been run against a real Ymir node or across real machines.</p>
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
      prose(_html),
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
