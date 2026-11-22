// LF-33 widget test, through the real bridge and bragi-core with the in-process demo node:
// the Theme screen (the drawer's Theme item) previews a theme live across the app, Use saves the
// choice, the editor changes a copy of a built-in with a live preview and saves it, and the
// choice survives a restart (a new controller reads it back).
//
//   BRAGI_BRIDGE_LIB=$CARGO_TARGET_DIR/debug/libbragi_bridge.so flutter test test/theme_test.dart --dart-define=BRAGI_DEMO=true
import 'dart:io';
import 'dart:ui' as ui;

import 'package:bragi/src/api.dart';
import 'package:bragi/src/app.dart';
import 'package:bragi/src/controller.dart';
import 'package:bragi/src/rust/api/bragi.dart' as rust;
import 'package:bragi/src/rust/frb_generated.dart';
import 'package:bragi/src/theme.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

void main() {
  late Directory dir;

  setUpAll(() async {
    final lib = Platform.environment['BRAGI_BRIDGE_LIB'];
    await RustLib.init(externalLibrary: lib == null ? null : ExternalLibrary.open(lib));
    dir = Directory.systemTemp.createTempSync('bragi-theme-test');
  });

  tearDownAll(() async {
    await rust.shutdown();
    dir.deleteSync(recursive: true);
  });

  testWidgets('theme: pick with a live preview, use, edit a copy, persist', (tester) async {
    tester.view.physicalSize = const Size(900, 1800);
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
    Future<void> tap(String key) async {
      await tester.tap(find.byKey(Key(key)));
      await tester.pump();
    }

    Future<void> settle() async {
      for (var i = 0; i < 12; i++) {
        await tester.runAsync(() => Future<void>.delayed(const Duration(milliseconds: 40)));
        await tester.pump();
      }
      await tester.pumpAndSettle();
    }

    await until('the pairing screen', () => shown('show-code'));
    await tap('show-code');
    await until('the check code', () => shown('codes-match'));
    await tap('codes-match');
    await until('the models', () => c.linked && c.models.isNotEmpty);

    // Bragi's own look to begin with.
    expect(c.appearance.pref.theme, 'bragi');
    expect(BragiColors.accent, const Color(0xFFF5B83D));

    // The menu item opens the screen.
    tester.state<ScaffoldState>(find.byType(Scaffold).first).openDrawer();
    await tester.pumpAndSettle();
    await tap('theme');
    await settle();
    expect(shown('theme-list'), isTrue);
    expect(shown('theme-nord') && shown('theme-dracula') && shown('theme-bragi'), isTrue);
    await tap('theme-mode');
    await shot('theme-1-list');

    // Tapping a theme shows it across the app without saving it.
    await tester.tap(find.text('Dark'));
    await settle();
    await tap('theme-nord');
    await settle();
    expect(BragiColors.background, isNot(const Color(0xFF14161B)));
    expect(c.appearance.pref.theme, 'bragi', reason: 'only previewed');
    expect(Theme.of(tester.element(find.byKey(const Key('theme-list')))).scaffoldBackgroundColor, BragiColors.background);
    await shot('theme-2-nord-preview');

    // Use saves it, with the mode.
    await tap('theme-use');
    await until('the saved choice', () => c.appearance.pref.theme == 'nord');
    expect(c.appearance.pref.mode, 'dark');
    expect(const RustBragiApi().themePreference().theme, 'nord');

    // Light mode previews the light variant; leaving without Use puts the saved look back.
    await tester.tap(find.text('Light'));
    await settle();
    final lightBg = BragiColors.background;
    expect(BragiColors.palette.dark, isFalse);
    await tester.pageBack();
    await settle();
    expect(BragiColors.palette.dark, isTrue, reason: 'the saved dark look is back');
    expect(BragiColors.background, isNot(lightBg));

    // Edit colours: a built-in is copied, then the role's colour is typed in and is live.
    tester.state<ScaffoldState>(find.byType(Scaffold).first).openDrawer();
    await tester.pumpAndSettle();
    await tap('theme');
    await settle();
    await tap('theme-menu-nord');
    await tester.pumpAndSettle();
    await tester.tap(find.text('Edit colours'));
    await settle();
    expect(shown('role-list'), isTrue);
    expect(find.text('nord-custom'), findsOneWidget);
    await tester.ensureVisible(find.byKey(const Key('role-accent')));
    await tap('role-accent');
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('hex-field')), '#ff0000');
    await tester.pump();
    expect(BragiColors.accent, const Color(0xFFFF0000), reason: 'live while typing');
    await shot('theme-3-editor');
    // A bad colour is flagged and changes nothing.
    await tester.enterText(find.byKey(const Key('hex-field')), 'zz');
    await tester.pump();
    expect(find.text('#rgb or #rrggbb'), findsOneWidget);
    expect(BragiColors.accent, const Color(0xFFFF0000));
    await tester.enterText(find.byKey(const Key('hex-field')), '#ff0000');
    await tester.pump();
    await tester.tapAt(const Offset(10, 10)); // closes the sheet
    await tester.pumpAndSettle();
    await tap('edit-save');
    await until('saved', () => find.textContaining('Saved nord-custom').evaluate().isNotEmpty);
    final saved = const RustBragiApi().themePalette('nord-custom', true);
    expect(saved.roles.firstWhere((r) => r.role == 'accent').hex, '#ff0000');
    expect(const RustBragiApi().themePalette('nord', true).roles.firstWhere((r) => r.role == 'accent').hex, isNot('#ff0000'), reason: 'the shipped nord is untouched');

    // Back in the list, use the copy; a restart (a new controller) reads the choice back.
    await tester.pageBack();
    await settle();
    await tap('theme-nord-custom');
    await settle();
    await tap('theme-use');
    await until('the copy chosen', () => c.appearance.pref.theme == 'nord-custom');
    final again = BragiController(const RustBragiApi());
    await tester.runAsync(again.appearance.load);
    expect(again.appearance.pref.theme, 'nord-custom');
    expect(BragiColors.accent, const Color(0xFFFF0000));
    await tester.pageBack();
    await settle();
    await shot('theme-4-saved');
  });
}
