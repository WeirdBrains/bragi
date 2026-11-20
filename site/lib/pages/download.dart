import 'package:jaspr/dom.dart';
import 'package:jaspr/jaspr.dart';

import '../components/layout.dart';
import '../site.dart';

const _rows = [
  (
    'Linux (x86_64, ARM64)',
    'Desktop, GPUI',
    'tar.gz, deb, rpm, Arch (TAUR), Nix',
    'Planned. The debug build runs from source.',
  ),
  ('Android', 'Phone, Flutter', 'apk, aab, F-Droid, Google Play', 'Planned. A debug APK builds from source.'),
  ('iOS', 'Phone, Flutter', 'TestFlight, App Store, AltStore', 'Planned (B3)'),
  ('macOS', 'Desktop, GPUI', 'zip, dmg', 'Planned (B3)'),
  ('Windows', 'Desktop, GPUI', 'zip, exe, installer', 'Planned (B3)'),
  ('FreeBSD and other BSDs', 'Desktop, GPUI', 'pkg, tar.gz', 'Planned where the toolkit builds (B5)'),
];

class DownloadPage extends StatelessComponent {
  const DownloadPage({super.key});

  @override
  Component build(BuildContext context) {
    return Component.fragment([
      pageHero(
        eyebrow: 'Download',
        title: 'Coming soon',
        lede: 'Nothing is released yet. These are the platforms and formats planned; none has a download.',
      ),
      div(classes: 'doc wrap', [
        table(classes: 'downloads', [
          thead([
            tr([
              th([Component.text('Platform')]),
              th([Component.text('App')]),
              th([Component.text('Formats')]),
              th([Component.text('Status')]),
            ]),
          ]),
          tbody([
            for (final (os, app, formats, status) in _rows)
              tr([
                td([Component.text(os)]),
                td([Component.text(app)]),
                td([Component.text(formats)]),
                td([
                  span(classes: 'badge planned', [Component.text(status)]),
                ]),
              ]),
          ]),
        ]),
        p([
          Component.text('Build from source in the meantime: see the '),
          a(href: '$repoUrl#building', [Component.text('README')]),
          Component.text('. Bragi needs a Ymir cloud to talk to; get that from '),
          a(href: ymirSiteUrl, [Component.text('Ymir')]),
          Component.text('.'),
        ]),
      ]),
    ]);
  }
}
