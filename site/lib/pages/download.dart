import 'package:jaspr/dom.dart';
import 'package:jaspr/jaspr.dart';

import '../components/layout.dart';
import '../site.dart';

/// One row of the download table: platform, app, the files (label and asset name), and a note.
typedef _Row = (String, String, List<(String, String)>, String);

final List<_Row> _rows = [
  (
    'Linux x86_64',
    'Desktop, GPUI',
    [
      ('tar.gz', 'bragi-$latestVersion-x86_64-unknown-linux-gnu.tar.gz'),
      ('zip', 'bragi-$latestVersion-x86_64-unknown-linux-gnu.zip'),
      ('deb', 'bragi_$latestVersion-1_amd64.deb'),
      ('rpm', 'bragi-$latestVersion-1.x86_64.rpm'),
      ('Arch', 'bragi-$latestVersion-1-x86_64.pkg.tar.zst'),
    ],
    'Debian, Ubuntu, Fedora, Arch and others. Standalone packages without the desktop entry: bragi-standalone.',
  ),
  (
    'Linux ARM64',
    'Desktop, GPUI',
    [
      ('tar.gz', 'bragi-$latestVersion-aarch64-unknown-linux-gnu.tar.gz'),
      ('zip', 'bragi-$latestVersion-aarch64-unknown-linux-gnu.zip'),
      ('deb', 'bragi_$latestVersion-1_arm64.deb'),
      ('rpm', 'bragi-$latestVersion-1.aarch64.rpm'),
      ('Arch', 'bragi-$latestVersion-1-aarch64.pkg.tar.zst'),
    ],
    'Standalone packages without the desktop entry: bragi-standalone.',
  ),
  (
    'Linux, terminal only',
    'Terminal app',
    [
      ('tar.gz, x86_64', 'bragi-tui-$latestVersion-x86_64-unknown-linux-gnu.tar.gz'),
      ('tar.gz, ARM64', 'bragi-tui-$latestVersion-aarch64-unknown-linux-gnu.tar.gz'),
      ('deb, x86_64', 'bragi-tui_$latestVersion-1_amd64.deb'),
      ('deb, ARM64', 'bragi-tui_$latestVersion-1_arm64.deb'),
      ('rpm, x86_64', 'bragi-tui-$latestVersion-1.x86_64.rpm'),
      ('Arch, x86_64', 'bragi-tui-$latestVersion-1-x86_64.pkg.tar.zst'),
    ],
    'The same bragi without GUI libraries, for servers and SSH. The desktop packages include it too: bragi with no '
        'arguments is the terminal app, bragi desktop opens the window.',
  ),
  (
    'macOS (Apple silicon and Intel)',
    'Desktop, GPUI',
    [
      ('dmg', 'bragi-$latestVersion-universal-apple-darwin.dmg'),
      ('zip', 'bragi-$latestVersion-universal-apple-darwin.zip'),
      ('binary, tar.gz', 'bragi-$latestVersion-universal-apple-darwin.tar.gz'),
    ],
    'Not notarized: the first time, right-click Bragi and choose Open.',
  ),
  (
    'Windows x86_64',
    'Desktop, GPUI',
    [
      ('Installer', 'bragi-$latestVersion-x86_64-pc-windows-msvc-setup.exe'),
      ('msi', 'bragi-$latestVersion-x86_64-pc-windows-msvc.msi'),
      ('Standalone exe', 'bragi-$latestVersion-x86_64-pc-windows-msvc.exe'),
      ('zip', 'bragi-$latestVersion-x86_64-pc-windows-msvc.zip'),
    ],
    'Installs for your user only, no admin needed. Not code-signed, so SmartScreen may warn.',
  ),
  (
    'Windows ARM64',
    'Desktop, GPUI',
    [
      ('Installer', 'bragi-$latestVersion-aarch64-pc-windows-msvc-setup.exe'),
      ('msi', 'bragi-$latestVersion-aarch64-pc-windows-msvc.msi'),
      ('Standalone exe', 'bragi-$latestVersion-aarch64-pc-windows-msvc.exe'),
      ('zip', 'bragi-$latestVersion-aarch64-pc-windows-msvc.zip'),
    ],
    'Not code-signed, so SmartScreen may warn.',
  ),
  (
    'Android',
    'Phone, Flutter',
    [
      ('apk, arm64', 'bragi-$latestVersion-android-arm64-v8a$androidSuffix.apk'),
      ('apk, armv7', 'bragi-$latestVersion-android-armeabi-v7a$androidSuffix.apk'),
      ('apk, x86_64', 'bragi-$latestVersion-android-x86_64$androidSuffix.apk'),
    ],
    'Sideload: most phones need the arm64 apk. Signed with a debug key, so uninstall before installing a newer '
        'version. Not on F-Droid or Google Play.',
  ),
  (
    'iOS',
    'Phone, Flutter',
    [
      ('ipa, unsigned', 'bragi-$latestVersion-ios-unsigned.ipa'),
    ],
    'Sideload with AltStore, SideStore or Sideloadly, which sign it with your own Apple ID. Not on TestFlight or the App '
        'Store.',
  ),
];

class DownloadPage extends StatelessComponent {
  const DownloadPage({super.key});

  @override
  Component build(BuildContext context) {
    return Component.fragment([
      pageHero(
        eyebrow: 'Download',
        title: 'Bragi $latestVersion',
        lede: 'Released $latestDate. Pick your platform. Bragi needs a Ymir cloud to talk to.',
      ),
      div(classes: 'doc wrap', [
        table(classes: 'downloads', [
          thead([
            tr([
              th([Component.text('Platform')]),
              th([Component.text('App')]),
              th([Component.text('Files')]),
              th([Component.text('Notes')]),
            ]),
          ]),
          tbody([
            for (final (os, app, files, note) in _rows)
              tr([
                td([Component.text(os)]),
                td([Component.text(app)]),
                td([
                  for (final (i, (label, name)) in files.indexed) ...[
                    if (i > 0) Component.text(' · '),
                    a(href: releaseAsset(name), [Component.text(label)]),
                  ],
                ]),
                td([Component.text(note)]),
              ]),
          ]),
        ]),
        p([
          Component.text('Arch Linux, from '),
          a(href: '$taurUrl/packages/bragi-bin', [Component.text('TAUR')]),
          Component.text(': '),
          code([Component.text(taurInstall)]),
          Component.text('. Nix: '),
          code([Component.text('nix run github:WeirdBrains/bragi')]),
          Component.text(' (or '),
          code([Component.text('.#bragi-standalone')]),
          Component.text('). Every file and its SHA-256 is on the '),
          a(href: releaseUrl, [Component.text('release page')]),
          Component.text('.'),
        ]),
        p([
          Component.text('Next: get Bragi linked to your cloud with the '),
          a(href: siteLink('/docs/guides/getting-started'), [Component.text('getting-started guide')]),
          Component.text('. Build from source instead: see the '),
          a(href: '$repoUrl#building', [Component.text('README')]),
          Component.text('. Ymir itself comes from '),
          a(href: ymirSiteUrl, [Component.text('Ymir')]),
          Component.text('.'),
        ]),
      ]),
    ]);
  }
}
