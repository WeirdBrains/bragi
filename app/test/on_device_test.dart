// B4: the On-device model settings: off by default, a model downloads only after its consent
// dialog gets an OK, progress shows while it downloads, and the picker is told when the model
// becomes ready. A fake API; no bridge, no real download.
import 'package:bragi/src/api.dart';
import 'package:bragi/src/on_device_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_ui/material_ui.dart';

class _OnDeviceApi implements BragiApi {
  bool enabled = false;
  bool installed = false;
  bool chosen = false;
  String? downloading;
  double progress = 0;
  final downloads = <(String, bool)>[];
  bool cancelled = false;

  OnDeviceView _view() => OnDeviceView(
    enabled: enabled,
    model: chosen ? 'tiny' : null,
    modelLabel: chosen ? 'Tiny' : null,
    installed: installed && chosen,
    engine: 'fake engine',
    downloading: downloading,
    progress: progress,
    progressLabel: downloading == null ? '' : '${(progress * 100).round()} of 100 bytes',
    error: null,
    ready: enabled && installed && chosen,
    line: !enabled
        ? 'Off. Nothing is downloaded or run on this device.'
        : installed
        ? 'Tiny, ready (fake engine).'
        : 'Tiny: not downloaded yet.',
  );

  @override
  OnDeviceView onDeviceStatus() => _view();

  @override
  OnDeviceView setOnDevice(bool enabled) {
    this.enabled = enabled;
    if (enabled) chosen = true;
    return _view();
  }

  @override
  List<OnDeviceModelView> onDeviceModels() => [
    OnDeviceModelView(
      id: 'tiny',
      label: 'Tiny',
      sizeLabel: '100 bytes',
      installed: installed,
      partial: false,
      chosen: chosen,
    ),
  ];

  @override
  List<String> onDeviceConsent(String id) => ['Download Tiny (100 bytes)', 'from https://hub.test/tiny'];

  @override
  OnDeviceView downloadOnDeviceModel(String id, bool consent) {
    downloads.add((id, consent));
    downloading = id;
    return _view();
  }

  @override
  bool cancelOnDeviceDownload() {
    cancelled = true;
    return true;
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => throw UnimplementedError('${invocation.memberName}');
}

Widget _wrap(BragiApi api, {Future<void> Function()? onChanged}) => MaterialApp(
  home: Scaffold(
    body: SingleChildScrollView(
      child: OnDeviceSection(api: api, onChanged: onChanged, poll: const Duration(milliseconds: 50)),
    ),
  ),
);

void main() {
  testWidgets('off by default, with no models listed', (t) async {
    final api = _OnDeviceApi();
    await t.pumpWidget(_wrap(api));
    expect(t.widget<SwitchListTile>(find.byKey(const Key('on-device'))).value, isFalse);
    expect(find.text('Off. Nothing is downloaded or run on this device.'), findsOneWidget);
    expect(find.byKey(const Key('on-device-model-tiny')), findsNothing);
  });

  testWidgets('a download waits for the consent dialog and Cancel downloads nothing', (t) async {
    final api = _OnDeviceApi();
    await t.pumpWidget(_wrap(api));
    await t.tap(find.byKey(const Key('on-device')));
    await t.pump();
    expect(find.text('Tiny: not downloaded yet.'), findsOneWidget);
    expect(find.text('100 bytes · not downloaded · in use'), findsOneWidget);

    await t.tap(find.byKey(const Key('on-device-get-tiny')));
    await t.pumpAndSettle();
    expect(find.byKey(const Key('on-device-consent')), findsOneWidget);
    expect(find.text('from https://hub.test/tiny'), findsOneWidget);
    await t.tap(find.text('Cancel'));
    await t.pumpAndSettle();
    expect(api.downloads, isEmpty);
  });

  testWidgets('OK downloads with progress, then the picker hears it is ready', (t) async {
    final api = _OnDeviceApi();
    var changed = 0;
    await t.pumpWidget(_wrap(api, onChanged: () async => changed++));
    await t.tap(find.byKey(const Key('on-device')));
    await t.pump();
    await t.tap(find.byKey(const Key('on-device-get-tiny')));
    await t.pumpAndSettle();
    await t.tap(find.byKey(const Key('on-device-ok')));
    await t.pump();
    expect(api.downloads, [('tiny', true)]);
    expect(find.byType(LinearProgressIndicator), findsOneWidget);
    expect(find.byKey(const Key('on-device-stop-tiny')), findsOneWidget);

    api.progress = 0.4;
    await t.pump(const Duration(milliseconds: 60));
    expect(find.text('40 of 100 bytes'), findsOneWidget);

    api
      ..downloading = null
      ..installed = true;
    await t.pump(const Duration(milliseconds: 60));
    expect(find.text('Tiny, ready (fake engine).'), findsOneWidget);
    expect(find.byType(LinearProgressIndicator), findsNothing);
    expect(find.byKey(const Key('on-device-remove-tiny')), findsOneWidget);
    expect(changed, 1);
  });

  testWidgets('Stop cancels the download', (t) async {
    final api = _OnDeviceApi()
      ..enabled = true
      ..chosen = true
      ..downloading = 'tiny';
    await t.pumpWidget(_wrap(api));
    await t.tap(find.byKey(const Key('on-device-stop-tiny')));
    await t.pump();
    expect(api.cancelled, isTrue);
    api.downloading = null;
    await t.pump(const Duration(milliseconds: 60));
  });
}
