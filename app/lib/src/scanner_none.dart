// The F-Droid build's stand-in for scanner.dart: no QR camera (mobile_scanner needs Google ML
// Kit, which F-Droid does not allow), so pairing with a desktop is by typing its code. The
// F-Droid recipe copies this file over scanner.dart; nothing imports it otherwise.
import 'package:material_ui/material_ui.dart';

/// Whether this build has a QR camera: never in this build.
bool get canScan => false;

/// Never shown, since [canScan] is false.
Widget cameraScanner(void Function(String) onCode) => const SizedBox.shrink();
