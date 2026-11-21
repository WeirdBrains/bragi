// The shared end-to-end scenario (pair, list models, chat, cancel, continue) through the
// real bridge and bragi-core, against the in-process demo node: no fakes, no real network.
//
//   cargo build -p bragi-bridge
//   BRAGI_BRIDGE_LIB=$CARGO_TARGET_DIR/debug/libbragi_bridge.so flutter test --dart-define=BRAGI_DEMO=true
//
// (The demo define makes the attach button add a sample file instead of opening the system picker.)
import 'dart:io';

import 'package:bragi/src/api.dart';
import 'package:bragi/src/app.dart';
import 'package:bragi/src/controller.dart';
import 'package:bragi/src/theme.dart';
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

    // B2: two inference nodes behind one model list.
    expect(c.state!.nodeCount, 2);
    expect(c.models.map((m) => m.id), containsAll(['gemma-4-e4b', 'llama-3.3-70b', 'qwen3-32b']));
    expect(c.models.firstWhere((m) => m.id == 'gemma-4-e4b').nodes, 2, reason: 'one row for a model on two nodes');
    expect(c.models.firstWhere((m) => m.id == 'llama-3.3-70b').nodes, 1);

    Future<void> ask(String text, int messages) async {
      await tester.enterText(find.byKey(const Key('prompt')), text);
      await tester.tap(find.byKey(const Key('send')));
      await until('the answer', () => !c.streaming && c.messages.length == messages && last().state == 'complete');
    }

    // A model that is not warm anywhere is answered by a stand-in, flagged.
    c.newChat();
    c.pickModel('qwen3-32b');
    await tester.pump();
    await ask('Why a stand-in?', 2);
    expect(last().standin, 'gemma-4-e4b');
    expect(last().model, 'qwen3-32b');
    expect(find.textContaining('stand-in gemma-4-e4b'), findsOneWidget);

    // The same question again is a cache hit.
    c.newChat();
    c.pickModel('gemma-4-e4b');
    await tester.pump();
    await ask('Same question?', 2);
    expect(last().cacheHit, isFalse);
    c.newChat();
    await tester.pump();
    await ask('Same question?', 2);
    expect(last().cacheHit, isTrue);
    expect(find.textContaining('cached'), findsOneWidget);

    // A node dies mid-answer; the other finishes it, exactly.
    c.newChat();
    await tester.pump();
    const dies = 'Keep going even if a node dies.';
    await ask(dies, 2);
    expect(last().resumes, 1);
    final wantDies = (await tester.runAsync(() => rust.demoAnswer(model: 'gemma-4-e4b', prompt: dies)))!;
    expect(last().text, wantDies);
    expect(find.textContaining('moved to another node'), findsOneWidget);

    // B5: a text file attached to a question reaches the model and stays with the message.
    c.newChat();
    await tester.pump();
    await tester.tap(find.byKey(const Key('attach')));
    await until('the file chip', () => shown('chip-0'));
    expect(c.attachments.single.name, 'sample-notes.md');
    await tester.enterText(find.byKey(const Key('prompt')), 'What is on my list?');
    await tester.tap(find.byKey(const Key('send')));
    await until('the answer', () => !c.streaming && c.messages.length == 2 && last().state == 'complete');
    expect(last().text, contains('I read sample-notes.md (3 lines)'));
    expect(c.messages.first.attachments.single.name, 'sample-notes.md');
    expect(c.attachments, isEmpty, reason: 'the chip went with the message');
    expect(find.textContaining('attached: sample-notes.md'), findsOneWidget);
    // A binary file is refused in the core's words.
    c.attachFile('pic.png', [0x89, 0x50, 0x00, 0x01]);
    expect(c.notice, contains('not a text file'));
    expect(c.attachments, isEmpty);

    // B5: agent mode. The node asks, the card is answered here, the tool runs on the node.
    c.newChat();
    await tester.pump();
    await tester.tap(find.byKey(const Key('agent')));
    await tester.pump();
    expect(c.agent, isTrue);
    await tester.enterText(find.byKey(const Key('prompt')), 'Find my notes about groceries.');
    await tester.tap(find.byKey(const Key('send')));
    await until('the approval card', () => shown('approval'));
    expect(c.card!.summary, contains('notes.search'));
    expect(c.card!.tier, 'navigate');
    await tester.tap(find.byKey(const Key('allow-once')));
    await until('the task', () => !c.streaming && c.messages.length == 2 && last().state == 'complete');
    expect(last().text, contains('2 notes mention'));
    expect(last().tools, ['notes.search']);
    expect(last().agent, isTrue);
    expect(find.textContaining('agent task · used notes.search'), findsOneWidget);
    expect(c.card, isNull);

    // Markdown: bragi-core parses the reply; bold, a list, a table and a code block render as
    // such, never with their markers, and the reply's copy button gives the raw Markdown.
    c.newChat();
    await tester.tap(find.byKey(const Key('agent')));
    await tester.pump();
    expect(c.agent, isFalse);
    await ask('Show me some Markdown.', 2);
    expect(last().text, contains('2 + 2 equals **4**.'));
    await tester.pump();
    expect(find.byKey(const Key('bubble-markdown')), findsOneWidget);
    expect(find.textContaining('**'), findsNothing, reason: 'no literal asterisks');
    expect(find.textContaining('2 + 2 equals 4.', findRichText: true), findsOneWidget);
    expect(find.byType(Table), findsOneWidget);
    expect(find.byKey(const Key('code-copy')), findsOneWidget);
    expect(find.textContaining('What Bragi renders', findRichText: true), findsOneWidget);
    final doc = c.markdown(last().text, false);
    expect((doc['blocks'] as List).map((b) => (b as Map)['type']), containsAll(['heading', 'list', 'table', 'code', 'quote']));
    expect((c.markdown('2 + 2 equals **4', true)['blocks'] as List).first['spans'][1], containsPair('bold', true));

    // A node address pasted into Link a device is kept and dialed (core: add_node), as on the
    // desktop; one without a peer id is refused in the core's words.
    const address = '/ip4/192.0.2.7/tcp/4848/p2p/12D3KooWQH96LGJd33JquZXNyrmrs7HAeaeviiFqdwnzwfdmj1VY';
    tester.state<ScaffoldState>(find.byType(Scaffold).first).openDrawer();
    await until('the drawer', () => shown('link-code'));
    await tester.pump(const Duration(milliseconds: 400));
    await tester.enterText(find.byKey(const Key('link-code')), '  $address ');
    await tester.tap(find.byKey(const Key('link-go')));
    await until('the address added', () => rust.peers().contains(address));
    expect(c.notice, isNull, reason: 'progress is not an error');
    // That address isn't one of the demo user's devices: said at once, as a warning.
    await until('the node search', () => c.nodeSearch == 'not_found');
    expect(c.nodeSearchLine, contains("isn't one of your linked devices"));
    expect(find.byKey(const Key('node-search')), findsOneWidget);
    expect(find.text(address), findsNothing, reason: 'the box is cleared');
    expect(await tester.runAsync(() => c.linkDevice('/ip4/192.0.2.7/tcp/4848')), isFalse);
    expect(c.notice, contains('peer'));
    expect(c.linkCheck, isNull, reason: 'an address is never opened as a link code');
    // Anything else the user does cleared the line (it arrives with the next state).
    await until('the line cleared', () => c.nodeSearchLine == null);

    // A linked node: looking, then "Connected to …" in the success colour (it used to stay red forever).
    final first = (await tester.runAsync(() async => rust.demoNodeAddress()))!;
    expect(await tester.runAsync(() => c.linkDevice(first)), isTrue);
    await until('connected', () => c.nodeSearch == 'found');
    expect(c.nodeSearchLine, 'Connected to demo inference node.');
    expect(c.notice, isNull);
    final line = tester.widget<Text>(find.byKey(const Key('node-search')));
    expect(line.style!.color, BragiColors.warm);
  });

  test('splice puts a delta at its offset', () {
    expect(splice('héllo world', 6, 'there'), 'héllo there');
    expect(splice('ab', 5, 'c'), 'abc');
  });
}
