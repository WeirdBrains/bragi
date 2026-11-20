/// Public URLs and copy used across the Bragi site.
library;

/// Where the site is served. Project pages live under `/bragi/`; a custom domain would be `/`.
/// Override at build time: `jaspr build --dart-define=SITE_BASE=/`.
const siteBase = String.fromEnvironment('SITE_BASE', defaultValue: '/bragi/');
const siteUrl = String.fromEnvironment('SITE_URL', defaultValue: 'https://weirdbrains.github.io/bragi');

/// A site-internal link that works under [siteBase].
String siteLink(String path) => '$siteBase${path.startsWith('/') ? path.substring(1) : path}';

const siteTitle = 'Bragi';
const siteTagline = 'Chat with your private AI cloud';
const siteDescription =
    'Bragi is the chat app for your own Ymir cloud, on desktop and phone. It pairs with your own devices over '
    'Weirdnet and streams answers from models on your own machines. Open source, BSD-3-Clause.';

const repoUrl = 'https://github.com/WeirdBrains/bragi';
const orgUrl = 'https://github.com/WeirdBrains';
const weirdnetUrl = 'https://net.weirdbrains.com';
const mainSiteUrl = 'https://www.weirdbrains.com';
// Sibling site: this is its GitHub Pages address until a custom domain exists.
const ymirSiteUrl = 'https://weirdbrains.github.io/ymir/';
const ymirRepoUrl = 'https://github.com/WeirdBrains/ymir';
String cratesIoUrl(String crate) => 'https://crates.io/crates/$crate';
