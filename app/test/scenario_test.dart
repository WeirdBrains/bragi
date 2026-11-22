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

    // A node address pasted into Link a device is kept and dialed (core: add_node), as on the
    // desktop; one without a peer id is refused in the core's words.
    const address = '/ip4/192.0.2.7/tcp/4848/p2p/12D3KooWQH96LGJd33JquZXNyrmrs7HAeaeviiFqdwnzwfdmj1VY';
    tester.state<ScaffoldState>(find.byType(Scaffold).first).openDrawer();
    await until('the drawer', () => shown('link-code'));
    await tester.pump(const Duration(milliseconds: 400));
    await tester.enterText(find.byKey(const Key('link-code')), '  $address ');
    await tester.tap(find.byKey(const Key('link-go')));
    await until('the address added', () => c.notice != null);
    expect(c.notice, startsWith("Added the node's address"));
    expect(rust.peers(), contains(address));
    expect(find.text(address), findsNothing, reason: 'the box is cleared');
    expect(await tester.runAsync(() => c.linkDevice('/ip4/192.0.2.7/tcp/4848')), isFalse);
    expect(c.notice, contains('peer'));
    expect(c.linkCheck, isNull, reason: 'an address is never opened as a link code');
  });

  testWidgets('sessions in folders, an endpoint session, and the Weirdnet fallback', (tester) async {
    tester.view.physicalSize = const Size(412, 915);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);

    // The demo from the first test is still running (starting it again is a no-op).
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

    Future<void> run(Future<void> Function() f) async {
      await tester.runAsync(f);
      await tester.pump();
    }

    bool shown(String key) => find.byKey(Key(key)).evaluate().isNotEmpty;

    await until('the chat', () => c.linked && c.route?.kind == 'weirdnet');
    expect(find.byKey(const Key('route')), findsOneWidget);
    expect(c.activeSessionId, 'weirdnet');
    final before = c.conversations.length;

    // A fake Ymir bridge on 127.0.0.1, added as an endpoint: checked, told apart as a bridge.
    final url = (await tester.runAsync(() => rust.startDemoBridge()))!;
    await run(() => c.addEndpoint('Demo bridge', url, rust.demoBridgeToken()));
    await until('the endpoint', () => c.endpoints.length == 1 && c.endpoints.single.health == 'ok');
    final ep = c.endpoints.single;
    expect(ep.kind, 'ymir_bridge');
    expect(ep.models, ['gemma-4-e4b']);

    // A folder with a session that sends to the bridge only.
    await run(() => c.createFolder('Projects'));
    await until('the folder', () => c.folders.length == 1);
    final folder = c.folders.single;
    await run(() => c.createSession('Bridge only', endpointId: ep.id, folder: folder.id));
    await until('the session', () => c.sessions.length == 2);
    final s = c.sessions.firstWhere((x) => x.name == 'Bridge only');
    expect(s.targetName, 'Demo bridge');
    expect(c.tree.map((r) => '${r.depth}:${r.kind}:${r.id}'), ['0:folder:${folder.id}', '1:session:${s.id}', '0:session:weirdnet']);

    // The drawer shows the tree; a tap on the session opens it.
    await tester.tap(find.byTooltip('Open navigation menu'));
    await tester.pumpAndSettle();
    expect(shown('folder-${folder.id}'), isTrue);
    expect(shown('session-${s.id}'), isTrue);
    await tester.tap(find.byKey(Key('session-${s.id}')));
    await until('the session to open', () => c.activeSessionId == s.id && c.route?.kind == 'endpoint');
    await tester.pumpAndSettle();
    expect(c.conversations, isEmpty, reason: 'a new session starts with no history');
    expect(find.textContaining('Sends to Demo bridge only'), findsOneWidget);

    // Its messages go to the bridge, and say so.
    const q = 'Which way did this go?';
    await tester.enterText(find.byKey(const Key('prompt')), q);
    await tester.tap(find.byKey(const Key('send')));
    await until('the answer', () => !c.streaming && c.messages.length == 2 && c.messages.last.state == 'complete');
    final want = (await tester.runAsync(() => rust.demoAnswer(model: 'gemma-4-e4b', prompt: q)))!;
    expect(c.messages.last.text, want);
    expect(c.messages.last.via, 'Demo bridge');
    expect(find.textContaining('via Demo bridge'), findsOneWidget);
    expect(c.conversations, hasLength(1));

    // Folding the folder hides the session; deleting the folder moves it up.
    await run(() => c.toggleFolder(folder.id));
    await until('the fold', () => c.tree.length == 2);
    await run(() => c.deleteFolder(folder.id));
    await until('the folder to go', () => c.folders.isEmpty);
    expect(c.session(s.id)!.folder, isNull);

    // Back in the Weirdnet session: its own history, and the bridge as its fallback.
    await run(() => c.switchSession('weirdnet'));
    await until('the Weirdnet session', () => c.activeSessionId == 'weirdnet');
    expect(c.conversations, hasLength(before));
    await run(() => c.setFallback(ep.id));
    await until('the fallback', () => c.endpoints.single.fallback);
    expect(c.route!.kind, 'weirdnet', reason: 'nodes are up, so Weirdnet first');
    expect(c.route!.detail, contains('Demo bridge as fallback'));

    // The endpoints screen shows it, without the token.
    await run(() => c.renameSession(s.id, 'Bridge prompts'));
    await until('the rename', () => c.session(s.id)!.name == 'Bridge prompts');
    expect(c.endpoints.single.toString(), isNot(contains(rust.demoBridgeToken())));

    // Settings: the endpoint is edited (its saved token is never shown) and tested.
    await tester.tap(find.byKey(const Key('endpoints')));
    await tester.pumpAndSettle();
    expect(find.textContaining('1 session'), findsOneWidget);
    await tester.tap(find.byKey(Key('endpoint-edit-${ep.id}')));
    await tester.pumpAndSettle();
    expect(tester.widget<TextField>(find.byKey(const Key('edit-token'))).controller!.text, isEmpty);
    await tester.enterText(find.byKey(const Key('edit-name')), 'Renamed bridge');
    await tester.tap(find.byKey(const Key('edit-test')));
    await until('the test', () => shown('edit-tested'));
    expect(find.textContaining('Ymir bridge · 1 model'), findsOneWidget);
    await tester.tap(find.byKey(const Key('edit-save')));
    await until('the save', () => c.endpoints.single.name == 'Renamed bridge' && !shown('edit-save'));
    await tester.pumpAndSettle();
    // A wrong token for a changed URL fails its test and changes nothing.
    await run(() => c.editEndpoint(ep.id, name: 'Renamed bridge', url: c.endpoints.single.url, token: 'wrong-token-000000000000', fallback: true));
    expect(c.notice, isNotNull);
    expect(c.endpoints.single.name, 'Renamed bridge');

    // Deleting it warns about its session and rebinds it to Weirdnet; the history stays.
    final all = c.conversations.length;
    await tester.tap(find.byKey(Key('endpoint-remove-${ep.id}')));
    await tester.pumpAndSettle();
    expect(find.textContaining('1 session sends to it'), findsOneWidget);
    await tester.tap(find.byKey(const Key('rebind-weirdnet')));
    await tester.tap(find.byKey(const Key('delete-endpoint-ok')));
    await until('the endpoint to go', () => c.endpoints.isEmpty);
    expect(c.session(s.id)!.endpointId, isNull);
    expect(c.conversations.length, all);

    // Links: another node's address is tested and added, renamed, then deleted after a confirm.
    final addr = (await tester.runAsync(() => rust.startDemoOtherNode()))!;
    await tester.enterText(find.byKey(const Key('link-name')), 'Other node');
    await tester.enterText(find.byKey(const Key('link-address')), addr);
    await tester.tap(find.byKey(const Key('link-add')));
    await until('the link', () => c.links.any((l) => l.address == addr));
    await tester.pumpAndSettle();
    await run(() => c.renameLink(addr, 'Desk node'));
    await until('the rename', () => c.links.any((l) => l.name == 'Desk node'));
    final bogus = addr.replaceFirst(RegExp(r'^/memory/\d+'), '/memory/987654321');
    await run(() => c.editLink(addr, bogus));
    expect(c.notice, isNotNull, reason: 'an address where nothing answers is refused');
    expect(c.links.any((l) => l.address == addr), isTrue);
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.byKey(Key('link-delete-$addr')));
    await tester.tap(find.byKey(Key('link-delete-$addr')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('delete-link-ok')));
    await until('the link to go', () => !c.links.any((l) => l.address == addr));
  });

  test('splice puts a delta at its offset', () {
    expect(splice('héllo world', 6, 'there'), 'héllo there');
    expect(splice('ab', 5, 'c'), 'abc');
  });
}
