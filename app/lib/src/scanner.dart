// The QR camera for pairing with a desktop (mobile_scanner). F-Droid builds can't ship it:
// mobile_scanner reads codes with Google ML Kit, which is not free software. The F-Droid recipe
// (packaging/fdroid) swaps this file for scanner_none.dart and drops mobile_scanner from
// pubspec.yaml, so those builds pair by typing the code instead. Keep the two files' API alike.
import 'dart:io';

import 'package:material_ui/material_ui.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

/// Whether this build has a QR camera: phones, in builds with mobile_scanner.
bool get canScan => Platform.isAndroid || Platform.isIOS;

/// The camera view; calls [onCode] with every code it reads.
Widget cameraScanner(void Function(String) onCode) => MobileScanner(
  onDetect: (capture) {
    for (final b in capture.barcodes) {
      final raw = b.rawValue;
      if (raw != null) onCode(raw);
    }
  },
);
