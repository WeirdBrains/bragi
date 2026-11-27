// Settings, On-device model (B4): off by default; each model downloads only after its consent
// dialog (what, how big, from where) gets an OK; a downloaded, chosen model shows in the target
// picker as "On this device". The core does the work (bragi_core::on_device); this asks, shows
// and polls while a download runs.
import 'dart:async';

import 'package:material_ui/material_ui.dart';

import 'api.dart';
import 'theme.dart';

class OnDeviceSection extends StatefulWidget {
  const OnDeviceSection({
    super.key,
    required this.api,
    this.onChanged,
    this.poll = const Duration(milliseconds: 500),
  });

  final BragiApi api;

  /// Called when the picker may change (on, off, a download done, a model removed).
  final Future<void> Function()? onChanged;

  /// How often the status is re-read while a model downloads.
  final Duration poll;

  @override
  State<OnDeviceSection> createState() => _OnDeviceSectionState();
}

class _OnDeviceSectionState extends State<OnDeviceSection> {
  late OnDeviceView _s = widget.api.onDeviceStatus();
  late List<OnDeviceModelView> _models = widget.api.onDeviceModels();
  String? _error;
  Timer? _timer;

  @override
  void initState() {
    super.initState();
    _follow();
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  void _follow() {
    _timer?.cancel();
    if (_s.downloading == null) return;
    _timer = Timer.periodic(widget.poll, (_) {
      final wasReady = _s.ready;
      final s = widget.api.onDeviceStatus();
      setState(() {
        _s = s;
        _models = widget.api.onDeviceModels();
      });
      if (s.downloading == null) {
        _timer?.cancel();
        if (s.ready != wasReady) widget.onChanged?.call();
      }
    });
  }

  void _do(OnDeviceView Function() f) {
    final wasReady = _s.ready;
    try {
      final s = f();
      setState(() {
        _s = s;
        _models = widget.api.onDeviceModels();
        _error = null;
      });
      _follow();
      if (s.ready != wasReady) widget.onChanged?.call();
    } catch (e) {
      setState(() => _error = '$e');
    }
  }

  Future<void> _download(OnDeviceModelView m) async {
    final List<String> lines;
    try {
      lines = widget.api.onDeviceConsent(m.id);
    } catch (e) {
      setState(() => _error = '$e');
      return;
    }
    final ok = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        key: const Key('on-device-consent'),
        title: const Text('Download this model?'),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [for (final l in lines) Text(l)],
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
          FilledButton(
            key: const Key('on-device-ok'),
            onPressed: () => Navigator.pop(context, true),
            child: const Text('Download'),
          ),
        ],
      ),
    );
    if (ok != true || !mounted) return;
    _do(() => widget.api.downloadOnDeviceModel(m.id, true));
  }

  Widget _model(OnDeviceModelView m) {
    final downloading = _s.downloading;
    final here = downloading == m.id;
    final state = m.installed
        ? 'downloaded'
        : m.partial
        ? 'partly downloaded'
        : 'not downloaded';
    final buttons = <Widget>[
      if (here)
        TextButton(
          key: Key('on-device-stop-${m.id}'),
          onPressed: () {
            widget.api.cancelOnDeviceDownload();
            setState(() {});
          },
          child: const Text('Stop'),
        ),
      if (!here && !m.installed && downloading == null)
        TextButton(
          key: Key('on-device-get-${m.id}'),
          onPressed: () => _download(m),
          child: const Text('Download'),
        ),
      if (m.installed && !m.chosen)
        TextButton(
          key: Key('on-device-use-${m.id}'),
          onPressed: () => _do(() => widget.api.chooseOnDeviceModel(m.id)),
          child: const Text('Use'),
        ),
      if (!here && (m.installed || m.partial))
        TextButton(
          key: Key('on-device-remove-${m.id}'),
          onPressed: () => _do(() => widget.api.removeOnDeviceModel(m.id)),
          child: const Text('Remove'),
        ),
    ];
    return ListTile(
      key: Key('on-device-model-${m.id}'),
      dense: true,
      title: Text(m.label),
      subtitle: here
          ? Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                LinearProgressIndicator(value: _s.progress),
                Text(_s.progressLabel, style: const TextStyle(fontSize: 12)),
              ],
            )
          : Text(
              '${m.sizeLabel} · $state${m.chosen ? ' · in use' : ''}',
              style: TextStyle(fontSize: 12, color: BragiColors.textSecondary),
            ),
      trailing: Row(mainAxisSize: MainAxisSize.min, children: buttons),
    );
  }

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SwitchListTile(
          key: const Key('on-device'),
          dense: true,
          title: const Text('Run a model on this device'),
          subtitle: Text(
            _error ?? _s.line,
            style: TextStyle(
              fontSize: 12,
              color: _error == null ? BragiColors.textSecondary : BragiColors.error,
            ),
          ),
          value: _s.enabled,
          onChanged: (on) => _do(() => widget.api.setOnDevice(on)),
        ),
        if (_s.enabled) for (final m in _models) _model(m),
      ],
    );
  }
}
