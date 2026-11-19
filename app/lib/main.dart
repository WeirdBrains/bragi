// Bragi on Android. `--dart-define=BRAGI_DEMO=true` runs against the in-process demo node
// with a test identity (nothing touches the real network).
import 'dart:io';

import 'package:material_ui/material_ui.dart';
import 'package:path_provider/path_provider.dart';

import 'src/api.dart';
import 'src/app.dart';
import 'src/controller.dart';
import 'src/rust/api/bragi.dart' as rust;
import 'src/rust/frb_generated.dart';

const demo = bool.fromEnvironment('BRAGI_DEMO');

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  final base = await getApplicationSupportDirectory();
  if (demo) {
    final dir = Directory('${base.path}/demo');
    if (dir.existsSync()) dir.deleteSync(recursive: true);
    await rust.startDemo(dir: dir.path, label: 'Bragi on Android (demo)', acceptAfterMs: 4000, wordDelayMs: 45);
  } else {
    await rust.start(dir: base.path, label: 'Bragi on Android');
  }
  final controller = BragiController(const RustBragiApi());
  await controller.attach();
  runApp(BragiApp(controller: controller));
}
