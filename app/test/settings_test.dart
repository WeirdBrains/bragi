// LF-11 widget tests, through the real bridge and bragi-core with the in-process demo node and a
// mock OpenAI-compatible server (dart:io) on a loopback port:
//   Settings: add / edit / test / reorder / delete endpoints (with the delete confirm and the rebind
//   flow), Weirdnet links (add / edit / delete); Sessions: target binding and nested folders.
//
//   BRAGI_BRIDGE_LIB=$CARGO_TARGET_DIR/debug/libbragi_bridge.so flutter test --dart-define=BRAGI_DEMO=true
import 'dart:convert';
import 'dart:io';

import 'package:bragi/src/api.dart';
import 'package:bragi/src/app.dart';
import 'package:bragi/src/controller.dart';
import 'package:bragi/src/rust/api/bragi.dart' as rust;
import 'package:bragi/src/rust/frb_generated.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

const secretKey = 'sk-widget-test-secret';

/// A tiny OpenAI-compatible server: `/v1/models` and a streamed `/v1/chat/completions`, both
/// needing `Authorization: Bearer secretKey`. It records the requests it served.
class MockOpenAi {
  late final HttpServer server;
  final seen = <String>[];

  Future<void> start() async {
    server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    server.listen((req) async {
      final auth = req.headers.value('authorization');
      seen.add('${req.method} ${req.uri.path} ${auth == null ? 'no-key' : 'key'}');
      if (auth != 'Bearer $secretKey') {
        req.response.statusCode = 401;
        req.response.headers.contentType = ContentType.json;
        req.response.write('{"error":{"message":"Incorrect API key provided"}}');
      } else if (req.uri.path.endsWith('/models')) {
        req.response.headers.contentType = ContentType.json;
        req.response.write(jsonEncode({'data': [{'id': 'mock-1'}, {'id': 'mock-2'}]}));
      } else {
        await utf8.decoder.bind(req).drain<void>();
        req.response.headers.contentType = ContentType('text', 'event-stream');
        for (final w in ['Hello ', 'from ', 'the mock']) {
          req.response.write('data: ${jsonEncode({'choices': [{'delta': {'content': w}, 'finish_reason': null}]})}\n\n');
        }
        req.response.write('data: ${jsonEncode({'choices': [{'delta': {}, 'finish_reason': 'stop'}]})}\n\ndata: [DONE]\n\n');
      }
      await req.response.close();
    });
  }

  String get url => 'http://127.0.0.1:${server.port}/v1';
}

void main() {
  late Directory dir;
  final mock = MockOpenAi();

  setUpAll(() async {
    final lib = Platform.environment['BRAGI_BRIDGE_LIB'];
    await RustLib.init(externalLibrary: lib == null ? null : ExternalLibrary.open(lib));
    dir = Directory.systemTemp.createTempSync('bragi-settings-test');
    await mock.start();
  });

  tearDownAll(() async {
    await rust.shutdown();
    await mock.server.close(force: true);
    dir.deleteSync(recursive: true);
  });

  testWidgets('settings and sessions: endpoints, links, targets, folders', (tester) async {
    tester.view.physicalSize = const Size(900, 2200);
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

    bool shown(String key) => find.byKey(Key(key)).evaluate().isNotEmpty;
    Future<void> tap(String key) async {
      await tester.tap(find.byKey(Key(key)));
      await tester.pump();
    }

    Future<void> type(String key, String text) async {
      await tester.enterText(find.byKey(Key(key)), text);
      await tester.pump();
    }

    /// Lets a core call and the redraw finish.
    Future<void> settle() async {
      await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 300)));
      await tester.pumpAndSettle();
    }

    // Pair, then open Settings from the drawer.
    await until('the pairing screen', () => shown('show-code'));
    await tap('show-code');
    await until('the check code', () => shown('codes-match'));
    await tap('codes-match');
    await until('the models', () => c.linked && c.models.isNotEmpty);

    // ---- Settings: add an endpoint (Test first, with a wrong key, then the right one) ----
    Future<void> openSettings() async {
      tester.state<ScaffoldState>(find.byType(Scaffold).first).openDrawer();
      await tester.pumpAndSettle();
      await tap('settings');
      await tester.pumpAndSettle();
    }

    await openSettings();
    expect(shown('no-endpoints'), isTrue);
    await tap('add-endpoint');
    await tester.pumpAndSettle();
    await type('ep-name', 'Mock A');
    await type('ep-url', mock.url);
    await type('ep-key', 'sk-wrong');
    await tap('ep-test');
    await until('the refusal', () => shown('ep-test-result'));
    expect(find.textContaining('refused the API key'), findsOneWidget);
    await type('ep-key', secretKey);
    await tap('ep-test');
    await until('a good test', () => find.textContaining('Connected; 2 models').evaluate().isNotEmpty);
    expect(find.byKey(const Key('ep-model')).evaluate().isNotEmpty, isTrue);
    expect(tester.widget<TextField>(find.byKey(const Key('ep-model'))).controller!.text, 'mock-1', reason: 'Test fills in the first model');
    await tap('ep-alongside');
    await tap('ep-save');
    await settle();
    expect(c.endpoints, hasLength(1));
    expect(c.endpoints.single.hasKey, isTrue);
    expect(c.endpoints.single.mode, 'alongside');
    expect(find.textContaining('key set'), findsOneWidget);
    expect(find.textContaining(secretKey), findsNothing, reason: 'the key is never drawn');
    final a = c.endpoints.single;

    // A bad form is refused with the core's words and nothing is added.
    await tap('add-endpoint');
    await tester.pumpAndSettle();
    await type('ep-name', 'Broken');
    await type('ep-url', 'not a url');
    await tap('ep-save');
    await settle();
    expect(shown('ep-error'), isTrue);
    expect(c.endpoints, hasLength(1));
    await tap('ep-cancel');
    await tester.pumpAndSettle();

    // A second one, fallback-only; then reorder and the order survives a reload.
    await tap('add-endpoint');
    await tester.pumpAndSettle();
    await type('ep-name', 'Mock B');
    await type('ep-url', mock.url);
    await type('ep-key', secretKey);
    await tap('ep-save');
    await settle();
    expect(c.endpoints.map((e) => e.name), ['Mock A', 'Mock B']);
    final b = c.endpoints[1];
    expect(find.textContaining('fallback #1'), findsOneWidget);
    await tap('endpoint-up-${b.id}');
    await settle();
    expect(c.endpoints.map((e) => e.name), ['Mock B', 'Mock A']);
    await tester.runAsync(c.reloadSettings);
    expect(c.endpoints.map((e) => e.name), ['Mock B', 'Mock A'], reason: 'the order is stored by the core');
    expect(tester.widget<IconButton>(find.byKey(Key('endpoint-up-${b.id}'))).onPressed, isNull, reason: 'first can not go up');

    // Test from the list; edit (rename, keep the key, off).
    await tap('endpoint-test-${a.id}');
    await until('the test snackbar', () => shown('endpoint-test-result'));
    expect(find.textContaining('Mock A: Connected; 2 models'), findsOneWidget);
    await tap('endpoint-edit-${a.id}');
    await tester.pumpAndSettle();
    expect(tester.widget<TextField>(find.byKey(const Key('ep-name'))).controller!.text, 'Mock A');
    expect(tester.widget<TextField>(find.byKey(const Key('ep-key'))).controller!.text, isEmpty, reason: 'the saved key is never shown');
    await type('ep-name', 'Mock Alpha');
    await tap('ep-enabled');
    await tap('ep-save');
    await settle();
    expect(c.endpoints.firstWhere((e) => e.id == a.id).name, 'Mock Alpha');
    expect(c.endpoints.firstWhere((e) => e.id == a.id).enabled, isFalse);
    expect(c.endpoints.firstWhere((e) => e.id == a.id).hasKey, isTrue, reason: 'an empty key field keeps the key');
    await tap('endpoint-edit-${a.id}');
    await tester.pumpAndSettle();
    await tap('ep-enabled');
    await tap('ep-save');
    await settle();

    // ---- Sessions: a chat bound to Mock Alpha, and one bound to Mock B (same server) ----
    Navigator.of(tester.element(find.byKey(const Key('settings-list')))).pop();
    await tester.pumpAndSettle();
    expect(c.targets.map((t) => t.label), ['Weirdnet pool', 'Mock Alpha']);
    Future<void> chatTo(TargetView? t, String text) async {
      c.newChat();
      if (t != null) c.pickTarget(t);
      await tester.pump();
      await tester.enterText(find.byKey(const Key('prompt')), text);
      await tester.tap(find.byKey(const Key('send')));
      await until('the answer', () => !c.streaming && c.messages.length == 2 && c.messages.last.state == 'complete');
    }

    await chatTo(c.targets[1], 'hello endpoint');
    expect(c.messages.last.text, 'Hello from the mock');
    expect(c.messages.last.node, 'Mock Alpha');
    expect(find.byKey(const Key('target-label')).evaluate().isNotEmpty, isTrue);
    expect(find.descendant(of: find.byKey(const Key('target-label')), matching: find.text('Mock Alpha · mock-1')), findsOneWidget);
    final onEndpoint = c.current!.id;
    await chatTo(null, 'hello pool');
    expect(c.messages.last.text, isNot('Hello from the mock'));
    final onPool = c.current!.id;
    await chatTo(c.targets[1], 'second on the endpoint');
    final second = c.current!.id;
    expect(c.sessions.where((s) => s.id == onEndpoint || s.id == second).every((s) => s.targetLabel == 'Mock Alpha · mock-1'), isTrue);
    expect(c.sessions.firstWhere((s) => s.id == onPool).targetLabel, 'Weirdnet pool');
    expect(mock.seen.where((s) => s.startsWith('POST') && s.endsWith('key')), hasLength(2), reason: 'only the bound chats used the endpoint');

    // ---- Delete with the confirm and the rebind flow ----
    await openSettings();
    await tap('endpoint-delete-${a.id}');
    await tester.pumpAndSettle();
    expect(find.textContaining('2 chats are bound to it'), findsOneWidget);
    expect(find.textContaining('Your chats and their history are kept'), findsOneWidget);
    await tap('ep-delete-cancel');
    await tester.pumpAndSettle();
    expect(c.endpoints, hasLength(2), reason: 'cancel deletes nothing');
    await tap('endpoint-delete-${a.id}');
    await tester.pumpAndSettle();
    await tap('rebind-0'); // Move to Weirdnet pool
    await tap('ep-delete-confirm');
    await settle();
    expect(c.endpoints.map((e) => e.name), ['Mock B']);
    expect(shown('endpoint-deleted'), isTrue);
    expect(find.textContaining('2 chats moved to Weirdnet pool'), findsOneWidget);
    expect(c.sessions.firstWhere((s) => s.id == onEndpoint).targetLabel, 'Weirdnet pool');
    expect((await tester.runAsync(() => c.api.conversation(onEndpoint)))!.messages, hasLength(2), reason: 'history is kept');

    // Bind a chat to Mock B (make it alongside first), delete B with "leave without a target".
    await tap('endpoint-edit-${b.id}');
    await tester.pumpAndSettle();
    await tap('ep-alongside');
    await tap('ep-save');
    await settle();
    Navigator.of(tester.element(find.byKey(const Key('settings-list')))).pop();
    await tester.pumpAndSettle();
    await chatTo(c.targets.last, 'bound to B');
    final onB = c.current!.id;
    await openSettings();
    await tap('endpoint-delete-${b.id}');
    await tester.pumpAndSettle();
    await tap('rebind-unbound');
    await tap('ep-delete-confirm');
    await settle();
    expect(c.endpoints, isEmpty);
    expect(shown('no-endpoints'), isTrue);
    final ub = c.sessions.firstWhere((s) => s.id == onB);
    expect(ub.unbound, isTrue);
    expect((await tester.runAsync(() => c.api.conversation(onB)))!.messages, hasLength(2), reason: 'history is kept');
    Navigator.of(tester.element(find.byKey(const Key('settings-list')))).pop();
    await tester.pumpAndSettle();
    await tester.runAsync(() => c.open(onB));
    await tester.pumpAndSettle();
    expect(find.textContaining('Its endpoint was deleted'), findsOneWidget);
    // Sending asks for a target; picking the pool rebinds it and it answers.
    await tester.enterText(find.byKey(const Key('prompt')), 'still there?');
    await tester.tap(find.byKey(const Key('send')));
    await settle();
    expect(c.notice, contains('endpoint was deleted'));
    await tap('target-menu');
    await tester.pumpAndSettle();
    await tap('target-0');
    await settle();
    expect(c.needsTarget, isFalse);
    await tester.enterText(find.byKey(const Key('prompt')), 'still there?');
    await tester.tap(find.byKey(const Key('send')));
    await until('the answer', () => !c.streaming && c.messages.length == 4 && c.messages.last.state == 'complete');

    // ---- Weirdnet links: add, edit, delete ----
    const addr1 = '/ip4/192.0.2.7/tcp/4848/p2p/12D3KooWQH96LGJd33JquZXNyrmrs7HAeaeviiFqdwnzwfdmj1VY';
    const addr2 = '/ip4/192.0.2.8/tcp/4848/p2p/12D3KooWQH96LGJd33JquZXNyrmrs7HAeaeviiFqdwnzwfdmj1VY';
    await openSettings();
    final before = c.links.length;
    await tap('add-link');
    await tester.pumpAndSettle();
    await type('link-address', 'nonsense');
    await tap('link-save');
    await settle();
    expect(shown('link-error'), isTrue);
    await type('link-address', addr1);
    await tap('link-save');
    await settle();
    expect(c.links.map((l) => l.address), contains(addr1));
    final idx = c.links.indexWhere((l) => l.address == addr1);
    await tap('link-edit-$idx');
    await tester.pumpAndSettle();
    await type('link-address', addr2);
    await tap('link-save');
    await settle();
    expect(c.links.map((l) => l.address), allOf(contains(addr2), isNot(contains(addr1))));
    final idx2 = c.links.indexWhere((l) => l.address == addr2);
    await tap('link-delete-$idx2');
    await tester.pumpAndSettle();
    await tap('link-delete-cancel');
    await tester.pumpAndSettle();
    expect(c.links.map((l) => l.address), contains(addr2));
    await tap('link-delete-$idx2');
    await tester.pumpAndSettle();
    await tap('link-delete-confirm');
    await settle();
    expect(c.links, hasLength(before));
    Navigator.of(tester.element(find.byKey(const Key('settings-list')))).pop();
    await tester.pumpAndSettle();

    // ---- Folders: nest, file a chat, move, delete (the chat stays) ----
    tester.state<ScaffoldState>(find.byType(Scaffold).first).openDrawer();
    await tester.pumpAndSettle();
    await tap('new-folder');
    await tester.pumpAndSettle();
    await type('folder-name', 'Work');
    await tap('folder-save');
    await settle();
    final work = c.folders.single;
    await tap('folder-menu-${work.id}');
    await tester.pumpAndSettle();
    await tap('folder-sub');
    await tester.pumpAndSettle();
    await type('folder-name', 'Reports');
    await tap('folder-save');
    await settle();
    final reports = c.folders.firstWhere((f) => f.name == 'Reports');
    expect(reports.parent, work.id);
    await tap('move-conv-$onPool');
    await tester.pumpAndSettle();
    await tap('pick-${reports.id}');
    await settle();
    expect(c.sessions.firstWhere((s) => s.id == onPool).folder, reports.id);
    expect(shown('conv-$onPool'), isTrue, reason: 'shown inside the open folders');
    // A folder can't go inside its own subfolder.
    await tap('folder-menu-${work.id}');
    await tester.pumpAndSettle();
    await tap('folder-move');
    await tester.pumpAndSettle();
    expect(shown('pick-${reports.id}'), isFalse, reason: 'its own subfolder is not offered');
    await tap('pick-top');
    await settle();
    // Collapsing hides what is inside; deleting the folder moves chats up and deletes none.
    await tap('folder-${reports.id}');
    await tester.pump();
    expect(shown('conv-$onPool'), isFalse);
    await tap('folder-menu-${reports.id}');
    await tester.pumpAndSettle();
    await tap('folder-delete');
    await tester.pumpAndSettle();
    expect(find.textContaining('No chat is deleted'), findsOneWidget);
    await tap('folder-delete-confirm');
    await settle();
    expect(c.folders.map((f) => f.name), ['Work']);
    expect(c.sessions.firstWhere((s) => s.id == onPool).folder, work.id, reason: 'moved up to the parent');
    expect((await tester.runAsync(() => c.api.conversation(onPool)))!.messages, hasLength(2));
  });
}
