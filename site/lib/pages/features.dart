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
<h2 id="training">Training, only when you say so</h2>
<p>No chat is used to train a model unless you mark it. The cap button in a chat's top bar (the Training button on the
desktop, <code>/train</code> in the terminal) marks it, after saying what that means, and marks it on all your linked
devices. Your Ymir nodes then export only the marked chats as training data for your own models, on your own devices,
leaving out system prompts, attachments, tool results and reasoning. Unmark it (<code>/train off</code>) and later
exports leave it out.</p>
<h2 id="modes">Chat and code, one conversation</h2>
<p>Chat and Code are modes of the same conversation, not separate screens. Plan in chat, switch to code with
<strong>Tab</strong> (or <code>/code</code>, <code>/chat</code>, <code>/mode</code>) and the plan comes along; switch back
and the code work is still there. Each message is marked with the mode it was sent in. Code mode adds a project folder,
files and folders as context (respecting <code>.gitignore</code>), suggested edits to review and apply, and shell commands
(<code>!cmd</code> or <code>/shell cmd</code>) whose output stays in the conversation. Its prompt fits the model's real
context window: files that don't fit are left out and named, and an over-long request is retried smaller instead of
failing with raw JSON.</p>
<h2 id="link">The desktop link</h2>
<p><code>bragi linker pair</code> shares project folders on your computer with Bragi on your phone. It shows a QR code
(and a code to type) to scan from the phone; after that the phone's code mode works on the desktop's folders, confined to
the folders you shared. The link runs over Weirdnet, directly on your network or through a relay, with no open ports.</p>
<h2 id="terminal">The terminal app</h2>
<p><code>bragi</code> with no arguments is Bragi in a terminal, locally or over SSH: the same conversations, models and
code mode. It keeps a prompt history (Up and Down), a list of past notifications (F9, <code>/notifications</code> or the
bell), collapsed thinking you can open again, and every list takes the mouse.</p>
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
        title: 'Pair, pick a model, chat and code',
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
