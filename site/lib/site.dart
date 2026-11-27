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

/// The latest release; bump with each release (docs/RELEASES.md).
const latestVersion = '0.17.0';
const latestDate = 'November 27, 2026';
const releaseUrl = '$repoUrl/releases/tag/v$latestVersion';
String releaseAsset(String name) => '$repoUrl/releases/download/v$latestVersion/$name';

/// The Android apks' name suffix: `-debug-signed` until a release key signs them.
const androidSuffix = '-debug-signed';
const taurUrl = 'https://pkg.taur.weirdbrains.com';

/// Installs bragi-bin from TAUR with yay, leaving yay's default AUR alone (TAUR docs/yay.md).
const taurInstall = "yay --aururl $taurUrl --aurrpcurl '$taurUrl/rpc?' -S bragi-bin";
const orgUrl = 'https://github.com/WeirdBrains';
const weirdnetUrl = 'https://net.weirdbrains.com';
const mainSiteUrl = 'https://weirdbrains.com';
// Sibling site. Until both custom domains are live this is its GitHub Pages address; the Pages
// workflow passes SIBLING_URL=https://ymir.weirdbrains.com/ when the CUSTOM_DOMAIN variable is set.
const ymirSiteUrl = String.fromEnvironment('SIBLING_URL', defaultValue: 'https://weirdbrains.github.io/ymir/');
const ymirRepoUrl = 'https://github.com/WeirdBrains/ymir';
String cratesIoUrl(String crate) => 'https://crates.io/crates/$crate';
