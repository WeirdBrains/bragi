// Code mode and the desktop link in the app:
//   - through the real bridge and bragi-core (the in-process demo node, a project folder in a
//     temp dir): the Chat/Code toggle, opening a project, picking context, a shell run, a code
//     ask saved in the same conversation, prompt history on Up/Down, and the bell's list;
//   - with a stand-in CodeApi: reviewing hunks and applying, and pairing by QR code or typed code.
//
//   BRAGI_BRIDGE_LIB=$CARGO_TARGET_DIR/debug/libbragi_bridge.so flutter test --dart-define=BRAGI_DEMO=true
import 'dart:io';

import 'package:bragi/src/api.dart';
import 'package:bragi/src/app.dart';
import 'package:bragi/src/code_api.dart';
import 'package:bragi/src/code_ui.dart';
import 'package:bragi/src/controller.dart';
import 'package:bragi/src/desktop_pair_screen.dart';
import 'package:bragi/src/rust/api/bragi.dart' as rust;
import 'package:bragi/src/rust/frb_generated.dart';
import 'package:flutter/services.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

/// A stand-in for the bridge's code calls: one proposed file with two hunks, and one desktop.
class FakeCode implements CodeApi {
  final paired = <String>[];
  final accepted = [true, true];
  var appliedCount = 0;
  var routeNow = 'auto';
  final kept = <NotificationView>[];

  ReviewView review() => ReviewView(
    explanation: 'Greets twice.',
    note: null,
    files: [
      FileDiffView(
        path: 'src/main.rs',
        created: false,
        added: 2,
        removed: 1,
        hunks: [
          for (final (i, a) in accepted.indexed)
            HunkView(
              header: '@@ -${i + 1} +${i + 1} @@',
              accepted: a,
              lines: [
                DiffLineView(
                  tag: 'removed',
                  text: 'old $i',
                  oldNo: i + 1,
                  newNo: null,
                ),
                DiffLineView(
                  tag: 'added',
                  text: 'new $i',
                  oldNo: null,
                  newNo: i + 1,
                ),
              ],
            ),
        ],
      ),
    ],
  );

  @override
  List<DesktopView> desktops() => [
    for (final p in paired)
      DesktopView(
        id: 'peer-$p',
        label: 'workstation',
        roots: [RootView(name: 'Projects', path: '~/Projects')],
        confirmShell: false,
        direct: ['x'],
        weirdnet: true,
        pairedMs: 0,
      ),
  ];
  @override
  Future<DesktopView> pair(String code, String label) async {
    if (!isPairCode(code)) throw 'not a pairing code';
    paired.add(code);
    return desktops().last;
  }

  @override
  bool isPairCode(String text) =>
      text.startsWith('weirdnet-bragi://desktop?') ||
      text.replaceAll('-', '').length == 28;
  @override
  Future<void> forget(String id) async =>
      paired.removeWhere((p) => 'peer-$p' == id);
  @override
  Future<DesktopView> refresh(String id) async => desktops().first;
  @override
  String route() => routeNow;
  @override
  void setRoute(String route) => routeNow = route;
  @override
  bool localAvailable() => false;
  @override
  Future<OpenedView> openLocal(String path) => throw UnimplementedError();
  @override
  Future<OpenedView> openDesktop(String id, int root, String sub) =>
      throw UnimplementedError();
  @override
  Future<void> close() async {}
  @override
  Future<List<EntryView>> list(String rel) async => const [];
  @override
  Future<ContextView> setContext(List<String> targets) =>
      throw UnimplementedError();
  @override
  Future<CodeAskView> ask(String? conversation, String prompt, String? model) =>
      throw UnimplementedError();
  @override
  Future<AnsweredView> answered(
    String? conversation,
    String prompt,
    String reply,
    String reasoning,
    String? model,
  ) => throw UnimplementedError();
  @override
  Future<ReviewView?> setHunk(int file, int hunk, bool accept) async {
    accepted[hunk] = accept;
    return review();
  }

  @override
  Future<AppliedView> apply() async {
    appliedCount = accepted.where((a) => a).length;
    return AppliedView(files: ['src/main.rs']);
  }

  @override
  Future<void> discard() async {}
  @override
  Future<AppliedView> undo() async => AppliedView(files: ['src/main.rs']);
  @override
  Stream<ShellEventView> shell(String id, String command) =>
      const Stream.empty();
  @override
  bool shellStop(String id) => false;
  @override
  Future<String> recordShell(String? conversation, String transcript) async =>
      'c';
  @override
  String? historyUp(String key, String current) => null;
  @override
  String? historyDown(String key) => null;
  @override
  void historyPush(String key, String prompt) {}
  @override
  void notify(String level, String text) =>
      kept.add(NotificationView(atMs: 0, level: level, text: text));
  @override
  List<NotificationView> notifications() => kept.reversed.toList();
  @override
  int unread() => kept.length;
  @override
  void seen() {}
}

void main() {
  group('with a stand-in code API', () {
    testWidgets('review hunks, drop one, apply', (tester) async {
      final fake = FakeCode();
      final chat = BragiController(const RustBragiApi(), code: fake);
      final code = chat.code;
      code.review = fake.review();
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: ListenableBuilder(
              listenable: code,
              builder: (_, _) => code.review == null
                  ? const Text('none')
                  : ReviewCard(code: code),
            ),
          ),
        ),
      );
      expect(find.text('src/main.rs'), findsOneWidget);
      expect(find.text('+ new 1'), findsOneWidget);
      await tester.tap(find.byKey(const Key('hunk-0-1')));
      await tester.pump();
      expect(fake.accepted, [true, false]);
      await tester.tap(find.byKey(const Key('review-apply')));
      await tester.pumpAndSettle();
      expect(fake.appliedCount, 1, reason: 'only the kept hunk is written');
      expect(code.applied, ['src/main.rs']);
      expect(find.text('none'), findsOneWidget);
      expect(fake.kept.last.text, 'Wrote src/main.rs');
    });

    testWidgets(
      'pair with desktop: a scanned QR code, then the typed code; unpair; route',
      (tester) async {
        final fake = FakeCode();
        final chat = BragiController(const RustBragiApi(), code: fake);
        final camera = <void Function(String)>[];
        await tester.pumpWidget(
          MaterialApp(
            home: DesktopPairScreen(
              code: chat.code,
              scanner: (onCode) {
                camera.add(onCode);
                return const ColoredBox(color: Colors.black);
              },
            ),
          ),
        );
        expect(find.text('None yet.'), findsOneWidget);
        await tester.tap(find.byKey(const Key('scan-qr')));
        await tester.pump();
        expect(find.byKey(const Key('scanner')), findsOneWidget);
        // Something else in view first, then the desktop's code (read many times a second).
        camera.last('https://example.com');
        camera.last('weirdnet-bragi://desktop?v=1&p=PEER&s=SECRET');
        camera.last('weirdnet-bragi://desktop?v=1&p=PEER&s=SECRET');
        await tester.pumpAndSettle();
        expect(fake.paired, [
          'weirdnet-bragi://desktop?v=1&p=PEER&s=SECRET',
        ], reason: 'one try per code');
        expect(find.text('workstation'), findsOneWidget);
        expect(
          find.byKey(const Key('scanner')),
          findsNothing,
          reason: 'the camera closes once a code is read',
        );

        await tester.enterText(
          find.byKey(const Key('pair-code')),
          'ABCD-EFGH-JKMN-PQRS-TVWX-YZ01-2345',
        );
        await tester.tap(find.byKey(const Key('pair-desktop')));
        await tester.pumpAndSettle();
        expect(fake.paired, hasLength(2));

        await tester.tap(find.text('Weirdnet first'));
        await tester.pump();
        expect(fake.routeNow, 'weirdnet');

        await tester.tap(find.byKey(Key('forget-peer-${fake.paired.first}')));
        await tester.pumpAndSettle();
        expect(fake.paired, hasLength(1));
      },
    );
  });

  group('through the bridge', () {
    late Directory dir;
    late Directory project;

    setUpAll(() async {
      final lib = Platform.environment['BRAGI_BRIDGE_LIB'];
      await RustLib.init(
        externalLibrary: lib == null ? null : ExternalLibrary.open(lib),
      );
      dir = Directory.systemTemp.createTempSync('bragi-code-test');
      project = Directory('${dir.path}/proj')..createSync();
      File('${project.path}/notes.md').writeAsStringSync('# Notes\nhello\n');
      Directory('${project.path}/src').createSync();
      File('${project.path}/src/lib.rs')
          .writeAsStringSync('pub fn one() -> u32 { 1 }\n');
    });

    tearDownAll(() async {
      await rust.shutdown();
      dir.deleteSync(recursive: true);
    });

    testWidgets(
      'toggle to code, open a project, context, shell, ask, history, bell',
      (tester) async {
        tester.view.physicalSize = const Size(412, 915);
        tester.view.devicePixelRatio = 1.0;
        addTearDown(tester.view.reset);

        await tester.runAsync(
          () => rust.startDemo(
            dir: '${dir.path}/home',
            label: 'Bragi test',
            acceptAfterMs: 200,
            wordDelayMs: 5,
          ),
        );
        final c = BragiController(const RustBragiApi());
        await tester.runAsync(c.attach);
        await tester.pumpWidget(BragiApp(controller: c));

        Future<void> until(String what, bool Function() done) async {
          for (var i = 0; i < 600; i++) {
            if (done()) return;
            await tester.runAsync(
              () => Future<void>.delayed(const Duration(milliseconds: 50)),
            );
            await tester.pump();
          }
          fail(
            'timed out waiting for $what (notice=${c.notice} messages=${c.messages.map((m) => '${m.mode}:${m.text}').toList()})',
          );
        }

        bool shown(String key) => find.byKey(Key(key)).evaluate().isNotEmpty;
        String box() => tester
            .widget<TextField>(find.byKey(const Key('prompt')))
            .controller!
            .text;

        await until('the pairing screen', () => shown('show-code'));
        await tester.tap(find.byKey(const Key('show-code')));
        await until('the check code', () => shown('codes-match'));
        await tester.tap(find.byKey(const Key('codes-match')));
        await until('the models', () => c.linked && shown('model-gemma-4-e4b'));

        // A chat message first: chat mode, with the attach and agent buttons.
        expect(shown('attach'), isTrue);
        await tester.enterText(
          find.byKey(const Key('prompt')),
          'What does Bragi do?',
        );
        await tester.tap(find.byKey(const Key('send')));
        await until(
          'the chat answer',
          () => !c.streaming && c.messages.length == 2,
        );

        // Code mode: the project bar replaces attach and agent; the conversation stays.
        await tester.tap(find.byKey(const Key('mode-code')));
        await tester.pump();
        expect(c.code.on, isTrue);
        expect(shown('attach'), isFalse);
        expect(shown('project'), isTrue);
        expect(find.text('Ask for a change…'), findsOneWidget);
        final conversation = c.current!.id;

        // A code ask with no project says what to do.
        await tester.enterText(find.byKey(const Key('prompt')), 'Add a two()');
        await tester.tap(find.byKey(const Key('send')));
        await tester.pump();
        expect(c.notice, contains('Open a project'));

        // Open the folder (the system folder picker is the user's; the controller call is the same).
        expect(
          await tester.runAsync(() => c.code.openLocal(project.path)),
          isTrue,
        );
        await tester.pump();
        expect(find.text(c.code.project!.place), findsOneWidget);

        // Context: browse into src/ and pick lib.rs.
        await tester.tap(find.byKey(const Key('context')));
        await until('the entries', () => shown('entry-notes.md'));
        await tester.pump(const Duration(seconds: 1)); // the sheet slides up
        expect(shown('entry-src'), isTrue);
        await tester.tap(find.byKey(const Key('open-src')));
        await until('src/', () => shown('entry-src/lib.rs'));
        await tester.tap(find.byKey(const Key('entry-src/lib.rs')));
        await tester.pump();
        await tester.tap(find.byKey(const Key('context-use')));
        await until('the context (${c.notice})', () => c.code.context != null);
        await tester.pump(const Duration(seconds: 1)); // the sheet slides away
        expect(shown('entries'), isFalse);
        expect(c.code.context!.files.map((f) => f.path), ['src/lib.rs']);
        expect(find.textContaining('1 file'), findsOneWidget);

        // A shell run streams into the panel and is saved in the conversation.
        await tester.runAsync(
          () => c.code.runShell('echo hello from the shell'),
        );
        await until(
          'the shell output',
          () => c.code.shell?.done == true && shown('shell-panel'),
        );
        expect(
          c.code.shell!.output.toString(),
          contains('hello from the shell'),
        );
        expect(find.text('exit 0'), findsOneWidget);
        await until(
          'the shell turn',
          () => c.messages.any(
            (m) => m.mode == 'code' && m.text.contains('hello from the shell'),
          ),
        );
        expect(c.current!.id, conversation);
        await tester.tap(find.byKey(const Key('shell-close')));
        await tester.pump();

        // A code ask: it streams, then is saved as a code turn of the same conversation.
        final before = c.messages.length;
        await tester.enterText(find.byKey(const Key('prompt')), 'Add a two()');
        await tester.tap(find.byKey(const Key('send')));
        await until(
          'the code answer',
          () =>
              !c.streaming &&
              c.messages.length == before + 2 &&
              c.messages.last.id.isNotEmpty,
        );
        expect(c.current!.id, conversation);
        expect(c.messages.last.mode, 'code');
        expect(c.messages.last.text, isNotEmpty);

        // Up and Down on a hardware keyboard: the code prompts, then back to the draft.
        await tester.tap(find.byKey(const Key('prompt')));
        await tester.enterText(find.byKey(const Key('prompt')), 'draft');
        await tester.sendKeyEvent(LogicalKeyboardKey.arrowUp);
        await tester.pump();
        expect(box(), 'Add a two()');
        await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
        await tester.pump();
        expect(box(), 'draft');

        // Chat mode keeps its own history.
        await tester.tap(find.byKey(const Key('mode-chat')));
        await tester.pump();
        await tester.enterText(find.byKey(const Key('prompt')), '');
        await tester.sendKeyEvent(LogicalKeyboardKey.arrowUp);
        await tester.pump();
        expect(box(), 'What does Bragi do?');

        // The bell lists what happened.
        expect(c.code.unread, greaterThan(0));
        await tester.tap(find.byKey(const Key('bell')));
        await tester.pumpAndSettle();
        expect(find.textContaining('Opened proj'), findsOneWidget);
        expect(c.code.unread, 0);
      },
    );
  });
}
