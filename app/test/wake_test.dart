// B4: the Wake-ups settings: push on and relay wake off by default, and relay wake asks first
// with its battery cost. A fake API; no bridge needed.
import 'package:bragi/src/api.dart';
import 'package:bragi/src/wake_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

class _WakeApi implements BragiApi {
  bool push = true;
  bool relay = false;
  final calls = <(bool, bool)>[];

  WakeView _view() => WakeView(push: push, relayWake: relay, relayConnected: false, refused: BigInt.zero);

  @override
  WakeView wakeStatus() => _view();

  @override
  WakeView setWakeSettings(bool push, bool relayWake) {
    calls.add((push, relayWake));
    this.push = push;
    relay = relayWake;
    return _view();
  }

  @override
  String relayWakeCost() => 'It uses more battery than push.';

  @override
  dynamic noSuchMethod(Invocation invocation) => throw UnimplementedError('${invocation.memberName}');
}

Widget _wrap(BragiApi api) => MaterialApp(
  home: Scaffold(body: WakeSection(api: api)),
);

bool _on(WidgetTester t, String key) => t.widget<SwitchListTile>(find.byKey(Key(key))).value;

void main() {
  testWidgets('push is on and relay wake off by default', (t) async {
    final api = _WakeApi();
    await t.pumpWidget(_wrap(api));
    expect(_on(t, 'wake-push'), isTrue);
    expect(_on(t, 'wake-relay'), isFalse);
  });

  testWidgets('relay wake shows its battery cost and stays off on Cancel', (t) async {
    final api = _WakeApi();
    await t.pumpWidget(_wrap(api));
    await t.tap(find.byKey(const Key('wake-relay')));
    await t.pumpAndSettle();
    expect(find.byKey(const Key('relay-wake-confirm')), findsOneWidget);
    expect(find.text('It uses more battery than push.'), findsOneWidget);
    await t.tap(find.text('Cancel'));
    await t.pumpAndSettle();
    expect(api.calls, isEmpty);
    expect(_on(t, 'wake-relay'), isFalse);

    await t.tap(find.byKey(const Key('wake-relay')));
    await t.pumpAndSettle();
    await t.tap(find.byKey(const Key('relay-wake-ok')));
    await t.pumpAndSettle();
    expect(api.calls, [(true, true)]);
    expect(_on(t, 'wake-relay'), isTrue);
  });

  testWidgets('push off turns relay wake off with it', (t) async {
    final api = _WakeApi()..relay = true;
    await t.pumpWidget(_wrap(api));
    await t.tap(find.byKey(const Key('wake-push')));
    await t.pumpAndSettle();
    expect(api.calls, [(false, true)]);
    expect(_on(t, 'wake-push'), isFalse);
    expect(_on(t, 'wake-relay'), isFalse, reason: 'relay wake needs push');
    expect(t.widget<SwitchListTile>(find.byKey(const Key('wake-relay'))).onChanged, isNull);
  });
}
