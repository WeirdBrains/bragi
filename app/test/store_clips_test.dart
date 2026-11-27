// Video follow-ups: Settings' "Play clips on all my devices" switch, off by default, saved through
// the core (`video.store_clip`). A fake API; no bridge needed.
import 'package:bragi/src/api.dart';
import 'package:bragi/src/settings_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

class _ClipsApi implements BragiApi {
  bool on = false;
  bool fail = false;
  final calls = <bool>[];

  @override
  bool videoStoreClip() => on;

  @override
  bool setVideoStoreClip(bool on) {
    calls.add(on);
    if (fail) throw 'the disk is full';
    this.on = on;
    return on;
  }

  @override
  int videoStoredClipMaxMib() => 25;

  @override
  dynamic noSuchMethod(Invocation invocation) => throw UnimplementedError('${invocation.memberName}');
}

Widget _wrap(BragiApi api) => MaterialApp(
  home: Scaffold(body: StoreClipsTile(api: api)),
);

bool _on(WidgetTester t) => t.widget<SwitchListTile>(find.byKey(const Key('store-clips'))).value;

void main() {
  testWidgets('off by default, with the cap in its text', (t) async {
    final api = _ClipsApi();
    await t.pumpWidget(_wrap(api));
    expect(_on(t), isFalse);
    expect(find.text('Play clips on all my devices'), findsOneWidget);
    expect(find.textContaining('up to 25 MiB'), findsOneWidget);
  });

  testWidgets('turning it on and off saves each time', (t) async {
    final api = _ClipsApi();
    await t.pumpWidget(_wrap(api));
    await t.tap(find.byKey(const Key('store-clips')));
    await t.pump();
    expect(api.calls, [true]);
    expect(_on(t), isTrue);
    await t.tap(find.byKey(const Key('store-clips')));
    await t.pump();
    expect(api.calls, [true, false]);
    expect(_on(t), isFalse);
  });

  testWidgets('shows the saved value, and a failed save says why and stays as it was', (t) async {
    final api = _ClipsApi()..on = true;
    await t.pumpWidget(_wrap(api));
    expect(_on(t), isTrue);
    api.fail = true;
    await t.tap(find.byKey(const Key('store-clips')));
    await t.pump();
    expect(_on(t), isTrue);
    expect(find.text('the disk is full'), findsOneWidget);
  });
}
