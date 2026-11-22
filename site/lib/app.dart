import 'package:jaspr/dom.dart';
import 'package:jaspr/jaspr.dart';
import 'package:jaspr_router/jaspr_router.dart';

import 'components/layout.dart';
import 'pages/docs.dart';
import 'pages/docs_sessions.dart';
import 'pages/docs_started.dart';
import 'pages/download.dart';
import 'pages/features.dart';
import 'pages/home.dart';
import 'pages/roadmap.dart';
import 'site.dart';

/// Every page of the site. Static mode renders each route to its own HTML file.
class App extends StatelessComponent {
  const App({super.key});

  @override
  Component build(BuildContext context) {
    return Router(
      routes: [
        Route(
          path: '/',
          title: '$siteTitle: $siteTagline',
          builder: (_, _) => const Layout(active: '/', child: Home()),
        ),
        Route(
          path: '/features',
          title: 'Features · $siteTitle',
          builder: (_, _) => const Layout(active: '/features', child: FeaturesPage()),
        ),
        Route(
          path: '/download',
          title: 'Download · $siteTitle',
          builder: (_, _) => const Layout(active: '/download', child: DownloadPage()),
        ),
        Route(
          path: '/docs',
          title: 'Docs · $siteTitle',
          builder: (_, _) => const Layout(active: '/docs', child: DocsPage()),
        ),
        Route(
          path: '/docs/guides/getting-started',
          title: 'Getting started · $siteTitle',
          builder: (_, _) => const Layout(active: '/docs', child: GettingStartedPage()),
        ),
        Route(
          path: '/docs/guides/sessions',
          title: 'Sessions and endpoints · $siteTitle',
          builder: (_, _) => const Layout(active: '/docs', child: SessionsPage()),
        ),
        Route(
          path: '/roadmap',
          title: 'Roadmap · $siteTitle',
          builder: (_, _) => const Layout(active: '/roadmap', child: RoadmapPage()),
        ),
      ],
      errorBuilder: (_, _) => Layout(
        active: '',
        child: section(classes: 'page-hero wrap', [
          h1([Component.text('Not found')]),
          p(classes: 'lede', [
            Component.text('That page is not here. '),
            a(href: siteLink('/'), [Component.text('Back home')]),
          ]),
        ]),
      ),
    );
  }
}
