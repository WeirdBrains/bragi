import 'package:jaspr/jaspr.dart';

import '../components/layout.dart';
import '../site.dart';

const _html = r'''
<h2 id="guides">Guides</h2>
<p>Guides for using Bragi.</p>
<ul>
<li><a href="${started}">Getting started</a>
  <ul>
  <li><a href="${started}#install">Install Bragi</a></li>
  <li><a href="${started}#link">Link Bragi to your Weirdnet</a></li>
  <li><a href="${started}#chat">Chat</a></li>
  </ul></li>
<li><a href="${mcp}">MCP server</a>: use Bragi from Cursor, Claude Desktop and VS Code</li>
<li><a href="${ymirGuide}">Ymir deployment guide</a>: set up the model cloud Bragi talks to, add a device, and connect Jan</li>
<li><a href="${features}">Features</a>: what Bragi does today</li>
</ul>
<p>Bragi is a <a href="https://weirdbrains.com">Weird Brains</a> project. Reference documentation and developer guides
for the core that Bragi shares with Ymir are on the <a href="${ymirDocs}">Ymir documentation</a> pages.</p>
''';

class DocsPage extends StatelessComponent {
  const DocsPage({super.key});

  @override
  Component build(BuildContext context) {
    return Component.fragment([
      pageHero(eyebrow: 'Docs', title: 'Documentation', lede: 'Guides to link Bragi and chat with your own models.'),
      prose(
        _html
            .replaceAll(r'${started}', siteLink('/docs/guides/getting-started'))
            .replaceAll(r'${mcp}', siteLink('/docs/guides/mcp'))
            .replaceAll(r'${features}', siteLink('/features'))
            .replaceAll(r'${ymirGuide}', '${ymirSiteUrl}docs/guides/deployment')
            .replaceAll(r'${ymirDocs}', '${ymirSiteUrl}docs'),
      ),
    ]);
  }
}
