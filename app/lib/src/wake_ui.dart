// Settings, Wake-ups (B4): push on by default with an off switch, and the opt-in relay wake
// path with its battery cost shown before it is turned on. The core does the work
// (bragi_core::wake); on Android the foreground service follows the relay wake switch.
import 'package:material_ui/material_ui.dart';

import 'api.dart';
import 'theme.dart';
import 'wake_service.dart';

class WakeSection extends StatefulWidget {
  const WakeSection({super.key, required this.api});

  final BragiApi api;

  @override
  State<WakeSection> createState() => _WakeSectionState();
}

class _WakeSectionState extends State<WakeSection> {
  late WakeView _w = widget.api.wakeStatus();
  String? _error;

  Future<void> _set(bool push, bool relayWake) async {
    try {
      final w = widget.api.setWakeSettings(push, relayWake);
      setState(() {
        _w = w;
        _error = null;
      });
      await syncWakeService(w.push && w.relayWake);
    } catch (e) {
      setState(() => _error = '$e');
    }
  }

  Future<void> _relay(bool on) async {
    if (on) {
      final ok = await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          key: const Key('relay-wake-confirm'),
          title: const Text('Wake through your relay?'),
          content: Text(widget.api.relayWakeCost()),
          actions: [
            TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('Cancel')),
            FilledButton(
              key: const Key('relay-wake-ok'),
              onPressed: () => Navigator.pop(context, true),
              child: const Text('Turn on'),
            ),
          ],
        ),
      );
      if (ok != true) return;
    }
    await _set(_w.push, on);
  }

  @override
  Widget build(BuildContext context) {
    final w = _w;
    final secondary = TextStyle(fontSize: 12, color: BragiColors.textSecondary);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SwitchListTile(
          key: const Key('wake-push'),
          dense: true,
          title: const Text('Wake-ups'),
          subtitle: Text(
            'Your other linked devices can wake this one to sync. A wake carries no message content. Off: reachable only while Bragi is open.',
            style: secondary,
          ),
          value: w.push,
          onChanged: (v) => _set(v, w.relayWake),
        ),
        SwitchListTile(
          key: const Key('wake-relay'),
          dense: true,
          title: const Text('Wake through your relay'),
          subtitle: Text(
            w.push
                ? (w.relayWake
                      ? (w.relayConnected
                            ? 'On: connected to your relay. Uses more battery.'
                            : 'On: connecting to your relay…')
                      : 'Off. Works without Google or Apple push, but keeps a connection open and uses more battery.')
                : 'Needs wake-ups on.',
            style: secondary,
          ),
          value: w.push && w.relayWake,
          onChanged: w.push ? _relay : null,
        ),
        if (_error != null)
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 16),
            child: Text(
              _error!,
              key: const Key('wake-error'),
              style: TextStyle(color: BragiColors.error),
            ),
          ),
      ],
    );
  }
}
