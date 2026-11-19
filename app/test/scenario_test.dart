// The shared end-to-end scenario (pair, list models, chat, cancel, continue) through the
// real bridge and bragi-core, against the in-process demo node: no fakes, no real network.
//
//   cargo build -p bragi-bridge
//   BRAGI_BRIDGE_LIB=$CARGO_TARGET_DIR/debug/libbragi_bridge.so flutter test
import 'dart:io';

import 'package:bragi/src/api.dart';
import 'package:bragi/src/app.dart';
import 'package:bragi/src/controller.dart';
import 'package:bragi/src/rust/api/bragi.dart' as rust;
import 'package:bragi/src/rust/frb_generated.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

const prompt1 = 'What does Bragi do?';
const prompt2 = 'Tell me more about cancel.';

void main() {
  late Directory dir;

  setUpAll(() async {
    final lib = Platform.environment['BRAGI_BRIDGE_LIB'];
    await RustLib.init(externalLibrary: lib == null ? null : ExternalLibrary.open(lib));
    dir = Directory.systemTemp.createTempSync('bragi-flutter-test');
  });

  tearDownAll(() async {
    await rust.shutdown();
    dir.deleteSync(recursive: true);
  });

  testWidgets('pair, list models, chat, cancel, continue', (tester) async {
    tester.view.physicalSize = const Size(412, 915);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    await tester.runAsync(() => rust.startDemo(dir: dir.path, label: 'Bragi test', acceptAfterMs: 200, wordDelayMs: 30));
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

    bool shown(String key) => find.byKey(Key(key)).evaluate().isNotEmpty;
    MessageView last() => c.messages.last;

    // Pair.
    await until('the pairing screen', () => shown('show-code'));
    await tester.tap(find.byKey(const Key('show-code')));
    await until('the check code', () => shown('codes-match'));
    expect(c.state!.pair.sas, hasLength(6));
    await tester.tap(find.byKey(const Key('codes-match')));

    // List models.
    await until('the models', () => c.linked && shown('model-gemma-4-e4b') && shown('model-qwen3-32b'));
    expect(c.model, 'gemma-4-e4b', reason: 'the warm model comes first');
    expect(c.models.map((m) => m.id), isNot(contains('nomic-embed-text')), reason: 'chat models only');

    // Chat.
    await tester.enterText(find.byKey(const Key('prompt')), prompt1);
    await tester.tap(find.byKey(const Key('send')));
    await until('the answer', () => !c.streaming && c.messages.length == 2 && last().state == 'complete');
    final want1 = (await tester.runAsync(() => rust.demoAnswer(model: 'gemma-4-e4b', prompt: prompt1)))!;
    expect(last().text, want1);

    // Cancel.
    await tester.enterText(find.byKey(const Key('prompt')), prompt2);
    await tester.tap(find.byKey(const Key('send')));
    await until('some streamed text', () => c.messages.length == 4 && last().text.split(' ').length > 8);
    expect(shown('stop'), isTrue);
    await tester.tap(find.byKey(const Key('stop')));
    await until('the stop', () => !c.streaming && last().state == 'stopped');
    final partial = last().text;
    final want2 = (await tester.runAsync(() => rust.demoAnswer(model: 'gemma-4-e4b', prompt: prompt2)))!;
    expect(partial.length, lessThan(want2.length));
    expect(shown('continue'), isTrue);

    // Continue.
    await tester.tap(find.byKey(const Key('continue')));
    await until('the rest', () => !c.streaming && last().state == 'complete');
    expect(last().text, want2);
    expect(last().text.startsWith(partial), isTrue);
    expect(c.conversations, hasLength(1));
  });

  test('splice puts a delta at its offset', () {
    expect(splice('héllo world', 6, 'there'), 'héllo there');
    expect(splice('ab', 5, 'c'), 'abc');
  });
}
