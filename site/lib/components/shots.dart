import 'package:jaspr/dom.dart';
import 'package:jaspr/jaspr.dart';

import '../site.dart';

/// A screenshot with a caption. All screenshots come from virtual-workspace runs of the demo profile.
Component shot(String file, String alt, String caption) => figure(classes: 'shot', [
  img(src: siteLink('images/screens/$file'), alt: alt),
  figcaption([Component.text(caption)]),
]);
