import 'package:jaspr/dom.dart';
import 'package:jaspr/jaspr.dart';

import '../components/shots.dart';
import '../content/roadmap.dart';
import '../site.dart';

Component _card(String title, String text) => div(classes: 'card', [
  h3([Component.text(title)]),
  p([Component.text(text)]),
]);

class Home extends StatelessComponent {
  const Home({super.key});

  @override
  Component build(BuildContext context) {
    final done = milestones.where((m) => m.status == 'done').map((m) => m.id).join(', ');
    return Component.fragment([
      section(classes: 'hero wrap', [
        img(src: siteLink('images/bragi-mark.svg'), alt: '', classes: 'hero-mark'),
        span(classes: 'eyebrow', [Component.text('Desktop · Phone · Open source')]),
        h1([Component.text('Chat with your private AI cloud')]),
        p(classes: 'lede', [
          Component.text(
            'Bragi is the chat app for your own Ymir cloud. It pairs with your devices over Weirdnet, finds the models '
            'on your own machines and streams their answers. Nothing goes through a hosted service.',
          ),
        ]),
        div(classes: 'actions', [
          a(href: siteLink('/features'), classes: 'button solid', [Component.text('What it does')]),
          a(href: siteLink('/docs/guides/getting-started'), classes: 'button', [Component.text('Get started')]),
          a(href: siteLink('/download'), classes: 'button', [Component.text('Download')]),
          a(href: siteLink('/roadmap'), classes: 'button', [Component.text('Roadmap')]),
        ]),
      ]),
      section(classes: 'section band', [
        div(classes: 'wrap', [
          div(classes: 'section-head', [
            h2([Component.text('The desktop app')]),
            p([
              Component.text(
                'Screenshots of the Linux desktop app on a demo profile with a simulated inference node '
                '(not a real Ymir cloud yet), captured in a virtual workspace.',
              ),
            ]),
          ]),
          div(classes: 'shots', [
            shot(
              'desktop-05-answer.png',
              'Bragi desktop showing a streamed answer',
              'Pick a model, ask, and watch the answer stream in.',
            ),
            shot(
              'desktop-03-check.png',
              'Bragi desktop showing the pairing check code',
              'Pairing shows a six-digit check that both devices must match.',
            ),
          ]),
        ]),
      ]),
      section(classes: 'section', [
        div(classes: 'wrap', [
          div(classes: 'section-head', [
            h2([Component.text('The phone app')]),
            p([
              Component.text(
                'The Android app on the same demo profile, run on a headless emulator. It does the same things '
                'through the same core.',
              ),
            ]),
          ]),
          div(classes: 'shots phones', [
            shot('phone-check.png', 'Bragi on Android showing the pairing check code', 'Pairing check.'),
            shot('phone-answer.png', 'Bragi on Android showing a streamed answer', 'A streamed answer.'),
            shot(
              'phone-stopped.png',
              'Bragi on Android with a stopped answer and a Continue button',
              'Stopped, with Continue.',
            ),
          ]),
        ]),
      ]),
      section(classes: 'section band', [
        div(classes: 'wrap', [
          div(classes: 'section-head', [
            h2([Component.text('How it fits')]),
            p([Component.text('One shared Rust core does all the work. The desktop and phone apps only draw it.')]),
          ]),
          div(classes: 'grid', [
            _card('Bragi', 'The chat client for desktop (GPUI, Linux first) and phone (Flutter, Android first).'),
            _card('Ymir', 'Your cloud of models across your own devices. Bragi asks it for models and prompts.'),
            _card('Weirdnet', 'The peer-to-peer network that links your devices. No open ports, no accounts.'),
          ]),
          div(classes: 'actions', [
            a(href: ymirSiteUrl, classes: 'button', [Component.text('Ymir')]),
            a(href: weirdnetUrl, classes: 'button', [Component.text('Weirdnet')]),
            a(href: repoUrl, classes: 'button', [Component.text('Source')]),
          ]),
          p(classes: 'roadmap-meta', [
            Component.text('Milestones done: $done. Nothing is released yet. '),
            a(href: siteLink('/roadmap'), [Component.text('See the roadmap.')]),
          ]),
        ]),
      ]),
    ]);
  }
}
