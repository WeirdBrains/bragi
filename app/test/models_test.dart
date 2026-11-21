// LF-30 widget test, through the real bridge and bragi-core with the in-process demo nodes (two
// model nodes of one owner, a mock Hugging Face Hub, the owner-only models/1 behind the
// roster): nodes and their status, search with a fit badge per node, pull with progress and
// cancel, auto, list, and remove behind a confirm.
//
//   BRAGI_BRIDGE_LIB=$CARGO_TARGET_DIR/debug/libbragi_bridge.so flutter test --dart-define=BRAGI_DEMO=true
import 'dart:io';
import 'dart:ui' as ui;

import 'package:bragi/src/api.dart';
import 'package:bragi/src/app.dart';
import 'package:bragi/src/controller.dart';
import 'package:bragi/src/models_ui.dart';
import 'package:bragi/src/rust/api/bragi.dart' as rust;
import 'package:bragi/src/rust/frb_generated.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

void main() {
  late Directory dir;

  setUpAll(() async {
    final lib = Platform.environment['BRAGI_BRIDGE_LIB'];
    await RustLib.init(externalLibrary: lib == null ? null : ExternalLibrary.open(lib));
    dir = Directory.systemTemp.createTempSync('bragi-models-test');
  });

  tearDownAll(() async {
    await rust.shutdown();
    dir.deleteSync(recursive: true);
  });

  test('sizes read like sentences', () {
    expect(humanBytes(512), '512 B');
    expect(humanBytes(4400000000), '4.4 GB');
    expect(statusLabel('needs_owner_device'), 'Needs owner device');
  });

  testWidgets('models: nodes, search with fit badges, pull with progress and cancel, auto, remove', (tester) async {
    tester.view.physicalSize = const Size(900, 2400);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    await tester.runAsync(() => rust.startDemo(dir: dir.path, label: 'Bragi test', acceptAfterMs: 200, wordDelayMs: 5));
    final c = BragiController(const RustBragiApi());
    await tester.runAsync(c.attach);
    await tester.pumpWidget(BragiApp(controller: c));

    Future<void> until(String what, bool Function() done) async {
      for (var i = 0; i < 600; i++) {
        if (done()) return;
        await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 50)));
        await tester.pump();
      }
      fail('timed out waiting for $what');
    }

    Future<void> shot(String name) async {
      final out = Platform.environment['BRAGI_SHOTS'];
      if (out == null) return;
      await tester.runAsync(() async {
        final boundary = tester.renderObject<RenderRepaintBoundary>(find.byType(RepaintBoundary).first);
        final image = await boundary.toImage();
        final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
        File('$out/$name.png').writeAsBytesSync(bytes!.buffer.asUint8List());
      });
    }

    bool shown(String key) => find.byKey(Key(key)).evaluate().isNotEmpty;
    bool text(String s) => find.textContaining(s).evaluate().isNotEmpty;
    Future<void> tap(String key) async {
      await tester.ensureVisible(find.byKey(Key(key)));
      await tester.tap(find.byKey(Key(key)));
      await tester.pump();
    }

    await until('the pairing screen', () => shown('show-code'));
    await tap('show-code');
    await until('the check code', () => shown('codes-match'));
    await tap('codes-match');
    await until('the models', () => c.linked && c.models.isNotEmpty);

    // Reach the Models screen from the drawer.
    tester.state<ScaffoldState>(find.byType(Scaffold).first).openDrawer();
    await tester.pumpAndSettle();
    await tap('models');
    await tester.pumpAndSettle();
    await until('both nodes ready', () => find.text('Ready').evaluate().length == 2);
    expect(shown('models-owner-banner'), isFalse, reason: 'a linked device is admitted');
    expect(text('No models downloaded.'), isTrue);
    await shot('flutter-51-models-nodes');

    // Search: a fit badge per node.
    await tester.enterText(find.byKey(const Key('models-search-field')), 'qwen');
    await tester.pump();
    await tap('models-search');
    await until('the results', () => shown('hit-mradermacher/DeepSeek-V4-Pro-Qwen3.5-9B-GGUF') && shown('hit-unsloth/Qwen3-14B-GGUF'));
    await until('every badge', () => !text('checking…'));
    expect(text('fits DeepSeek-V4-Pro-Qwen3.5-9B.Q8_0'), isTrue, reason: '24 GB holds the Q8');
    expect(text('fits DeepSeek-V4-Pro-Qwen3.5-9B.Q6_K'), isTrue, reason: '11 GB holds the Q6 only');
    expect(text('too big for 11.0 GB'), isTrue);
    await shot('flutter-52-models-search');

    // Pull the small repo's Q4 to all nodes, watch progress, cancel.
    await tester.enterText(find.byKey(const Key('models-search-field')), 'tiny');
    await tester.pump();
    await tap('models-search');
    await until('the tiny repo', () => shown('hit-demo/Tiny-Chat-GGUF') && !shown('hit-unsloth/Qwen3-14B-GGUF'));
    await until('its badges', () => !text('checking…'));
    await tester.tap(find.byKey(const Key('models-target')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('All nodes').last);
    await tester.pumpAndSettle();
    await tap('pull-demo/Tiny-Chat-GGUF');
    await until('progress', () => find.byWidgetPredicate((w) => w is LinearProgressIndicator && w.value != null && w.value! > 0).evaluate().isNotEmpty);
    expect(shown('pull-cancel'), isTrue);
    await shot('flutter-53-models-progress');
    await tap('pull-cancel');
    await until('the cancel', () => shown('pull-ended'));
    expect(text('Cancelled'), isTrue);
    await tap('pull-dismiss');
    await tester.pump();

    // Auto: the node where the biggest fits, and why; then it is listed and serving.
    await tester.tap(find.byKey(const Key('models-target')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Auto (best fit)').last);
    await tester.pumpAndSettle();
    await tap('pull-demo/Tiny-Chat-GGUF');
    await until('the auto choice', () => shown('pull-chosen'));
    expect(text('can hold the biggest one'), isTrue);
    await until('the pull to finish', () => shown('pull-ended') && text('downloaded and serving'));
    await until('it listed', () => text('Tiny-Chat-Q8_0 · 5.0 MB'));
    await shot('flutter-54-models-pulled');

    // Remove asks first; Cancel keeps it, Remove deletes it.
    final remove = find.byWidgetPredicate((w) => w.key.toString().contains('remove-') && w.key.toString().contains('Tiny-Chat-Q8_0'));
    expect(remove, findsOneWidget);
    await tester.ensureVisible(remove);
    await tester.tap(remove);
    await tester.pumpAndSettle();
    expect(shown('remove-dialog'), isTrue);
    await shot('flutter-55-models-remove-confirm');
    await tap('remove-cancel');
    await tester.pumpAndSettle();
    expect(text('Tiny-Chat-Q8_0 · 5.0 MB'), isTrue);
    await tester.tap(remove);
    await tester.pumpAndSettle();
    await tap('remove-confirm');
    await until('it gone', () => !text('Tiny-Chat-Q8_0 · 5.0 MB'));
    expect(text('5.0 MB freed'), isTrue);
    await shot('flutter-56-models-removed');
  });
}
