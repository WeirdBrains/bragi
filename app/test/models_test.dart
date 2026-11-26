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
import 'package:bragi/src/theme.dart';
import 'package:bragi/src/upload_ui.dart';
import 'package:bragi/src/rust/api/bragi.dart' as rust;
import 'package:bragi/src/rust/frb_generated.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

/// A device whose nodes all refuse it: only the models calls exist.
class _RefusedApi implements BragiApi {
  @override
  Future<List<NodeModelsView>> modelsOverview() async => [
    NodeModelsView(
      node: const ModelNodeView(peer: 'peer-a', label: 'forge'),
      status: 'needs_owner_device',
      detail: 'needs-admission',
      models: const [],
    ),
    NodeModelsView(
      node: const ModelNodeView(peer: 'peer-b', label: 'anvil'),
      status: 'needs_owner_device',
      detail: 'unknown-device',
      models: const [],
    ),
  ];

  @override
  Future<List<PendingUploadView>> modelsPendingUploads() async => const [];

  @override
  dynamic noSuchMethod(Invocation invocation) => throw UnimplementedError('${invocation.memberName}');
}

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
    expect(parsePercent(''), 0);
    expect(parsePercent(' 20 % '), 20);
    expect(parsePercent('100'), 100);
    expect(parsePercent('101'), isNull);
    expect(parsePercent('-1'), isNull);
    expect(parsePercent('half'), isNull);
  });

  testWidgets('a device the nodes refuse sees why, and nothing can change', (tester) async {
    tester.view.physicalSize = const Size(900, 1200);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    final c = BragiController(_RefusedApi());
    await tester.pumpWidget(MaterialApp(theme: bragiTheme(), home: ModelsScreen(controller: c)));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('models-owner-banner')), findsOneWidget);
    expect(find.text('Needs owner device'), findsNWidgets(2));
    expect(find.textContaining('needs-admission'), findsOneWidget);
    expect(tester.widget<FilledButton>(find.byKey(const Key('models-search'))).onPressed, isNull);
    expect(tester.widget<TextField>(find.byKey(const Key('models-search-field'))).enabled, isFalse);
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

    // Upload a model from this device: read and hashed first, then a node, license, a rollout
    // with a 20 % share; the same upload as a ymirctl command for a computer; then sent.
    final file = '${dir.path}/my-tune-Q8_0.gguf';
    await tester.runAsync(() => rust.demoWriteGguf(path: file, size: 2 << 20));
    await tap('models-upload');
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('upload-path')), file);
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await until('the preview', () => shown('upload-preview'));
    expect(text('model, GGUF'), isTrue);
    Future<void> pickNode(String label) async {
      await tester.ensureVisible(find.byKey(const Key('upload-node')));
      await tester.tap(find.byKey(const Key('upload-node')));
      await tester.pumpAndSettle();
      await tester.tap(find.text(label).last);
      await tester.pumpAndSettle();
    }

    await pickNode('demo inference node 2');
    await tester.enterText(find.byKey(const Key('upload-license')), 'apache-2.0');
    await tester.enterText(find.byKey(const Key('upload-rollout')), 'support');
    await tester.pump();
    await tap('upload-preset-20');
    expect(find.text('20'), findsOneWidget, reason: 'the preset fills the typed share');
    await tap('upload-show-command');
    await until('the command', () => shown('upload-command'));
    expect(text('ymirctl models upload ~/models/my-tune-Q8_0.gguf --node'), isTrue);
    expect(text('--license apache-2.0'), isTrue);
    expect(text('--canary 20'), isTrue);
    await shot('flutter-57-upload-dialog');
    await tester.enterText(find.byKey(const Key('upload-percent')), '120');
    await tap('upload-send');
    expect(text('a percentage from 0 to 100'), isTrue);
    await tester.enterText(find.byKey(const Key('upload-percent')), '20');
    // The second demo node keeps more free than its disk has: it refuses with no-space.
    await tap('upload-send');
    await until('the refusal', () => shown('upload-refused') || shown('upload-ended') || shown('upload-result'));
    expect(text('upload_keep_free'), isTrue, reason: 'the second node has no room');
    await pickNode('demo inference node');
    await tap('upload-send');
    await until('the upload', () => shown('upload-result') || shown('upload-refused') || shown('upload-ended'));
    String said(String key) => shown(key) ? tester.widget<Text>(find.byKey(Key(key))).data ?? '' : '';
    expect(shown('upload-result'), isTrue, reason: 'the demo node takes it: ${said('upload-refused')}${said('upload-ended')}');
    expect(text('Stored '), isTrue);
    expect(shown('upload-rollout-result') || shown('upload-rollout-error'), isTrue);
    final rolled = shown('upload-rollout-result');
    await shot('flutter-58-upload-done');
    await tester.pageBack();
    await tester.pumpAndSettle();
    await until('it listed', () => text('my-tune'));

    // The Training view: each demo node's finished run, and the rollout the upload made.
    await tap('models-training');
    await until('the runs', () => text('+demo'));
    if (rolled) expect(text('support on'), isTrue, reason: 'the rollout the upload made');
    expect(text('CC0-1.0'), isTrue);
    await tester.pumpAndSettle();
    await shot('flutter-60-training');
    await tester.pageBack();
    await tester.pumpAndSettle();
  });
}
