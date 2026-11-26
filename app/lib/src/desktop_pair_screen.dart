// Pair with desktop: scan the QR code `bragi linker pair` shows on the desktop (or type its
// short code), pick how to reach paired desktops, and unpair them. The link is end-to-end
// encrypted and pinned to the desktop's key whichever way it goes (docs/desktop-link.md).
import 'dart:io';

import 'package:material_ui/material_ui.dart';

import 'code_controller.dart';
import 'scanner.dart';
import 'theme.dart';

class DesktopPairScreen extends StatefulWidget {
  const DesktopPairScreen({super.key, required this.code, this.scanner});

  final CodeController code;

  /// The camera view (tests pass their own); it calls back with each code it reads.
  final Widget Function(void Function(String) onCode)? scanner;

  @override
  State<DesktopPairScreen> createState() => _DesktopPairScreenState();
}

class _DesktopPairScreenState extends State<DesktopPairScreen> {
  final _typed = TextEditingController();
  bool _scanning = false;
  bool _pairing = false;
  String? _last;

  @override
  void dispose() {
    _typed.dispose();
    super.dispose();
  }

  String get _label => switch (Platform.operatingSystem) {
    'android' => 'Bragi on Android',
    'ios' => 'Bragi on iPhone',
    final os => 'Bragi on $os',
  };

  Future<void> _pair(String text) async {
    if (_pairing || text.trim().isEmpty) return;
    setState(() {
      _pairing = true;
      _scanning = false;
    });
    final d = await widget.code.pair(text, _label);
    if (!mounted) return;
    setState(() => _pairing = false);
    if (d != null) {
      _typed.clear();
      ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text('Paired with ${d.label}.')));
    }
  }

  void _scanned(String raw) {
    // The camera reads the same code many times a second; one try per code.
    if (raw == _last || !widget.code.isPairCode(raw)) return;
    _last = raw;
    _pair(raw);
  }

  Widget _camera() {
    final custom = widget.scanner;
    if (custom != null) return custom(_scanned);
    return cameraScanner(_scanned);
  }

  @override
  Widget build(BuildContext context) {
    final c = widget.code;
    return ListenableBuilder(
      listenable: c,
      builder: (context, _) {
        final desktops = c.desktops;
        return Scaffold(
          appBar: AppBar(title: const Text('Pair with desktop')),
          body: ListView(
            padding: const EdgeInsets.all(16),
            children: [
              Text(
                'On the desktop run `bragi linker pair`, then scan the QR code it shows. '
                'The code works once, for five minutes.',
                style: TextStyle(color: BragiColors.textSecondary),
              ),
              const SizedBox(height: 12),
              if (_scanning)
                ClipRRect(
                  borderRadius: BorderRadius.circular(12),
                  child: SizedBox(key: const Key('scanner'), height: 300, child: _camera()),
                )
              else if (canScan || widget.scanner != null)
                FilledButton.icon(
                  key: const Key('scan-qr'),
                  onPressed: _pairing ? null : () => setState(() => _scanning = true),
                  icon: const Icon(Icons.qr_code_scanner),
                  label: const Text('Scan the QR code'),
                ),
              const SizedBox(height: 16),
              Text('Or type the code under it', style: TextStyle(color: BragiColors.textSecondary, fontSize: 12)),
              const SizedBox(height: 6),
              Row(
                children: [
                  Expanded(
                    child: TextField(
                      key: const Key('pair-code'),
                      controller: _typed,
                      autocorrect: false,
                      enableSuggestions: false,
                      textCapitalization: TextCapitalization.characters,
                      decoration: const InputDecoration(hintText: 'XXXX-XXXX-…', isDense: true, border: OutlineInputBorder()),
                      onSubmitted: _pair,
                    ),
                  ),
                  const SizedBox(width: 8),
                  FilledButton(
                    key: const Key('pair-desktop'),
                    onPressed: _pairing ? null : () => _pair(_typed.text),
                    child: _pairing ? const SizedBox(width: 16, height: 16, child: CircularProgressIndicator(strokeWidth: 2)) : const Text('Pair'),
                  ),
                ],
              ),
              const SizedBox(height: 24),
              const Text('Reach desktops', style: TextStyle(fontWeight: FontWeight.w600)),
              const SizedBox(height: 6),
              SegmentedButton<String>(
                key: const Key('link-route'),
                segments: const [
                  ButtonSegment(value: 'auto', label: Text('Auto')),
                  ButtonSegment(value: 'direct', label: Text('Direct only')),
                  ButtonSegment(value: 'weirdnet', label: Text('Weirdnet first')),
                ],
                selected: {c.route},
                onSelectionChanged: (s) => c.setRoute(s.first),
              ),
              const SizedBox(height: 4),
              Text(switch (c.route) {
                'direct' => 'Only on the same network or VPN as the desktop.',
                'weirdnet' => 'Through the Weirdnet relay first (end-to-end encrypted), then directly.',
                _ => 'Directly on the same network or VPN, else through the Weirdnet relay (end-to-end encrypted).',
              }, style: TextStyle(color: BragiColors.textSecondary, fontSize: 12)),
              const SizedBox(height: 24),
              const Text('Paired desktops', style: TextStyle(fontWeight: FontWeight.w600)),
              if (desktops.isEmpty)
                Padding(
                  padding: const EdgeInsets.symmetric(vertical: 8),
                  child: Text('None yet.', style: TextStyle(color: BragiColors.textSecondary)),
                ),
              for (final d in desktops)
                ListTile(
                  key: Key('desktop-${d.id}'),
                  contentPadding: EdgeInsets.zero,
                  leading: const Icon(Icons.computer),
                  title: Text(d.label),
                  subtitle: Text(
                    [
                      '${d.roots.length} shared folder${d.roots.length == 1 ? '' : 's'}',
                      if (d.direct.isNotEmpty) 'direct',
                      if (d.weirdnet) 'Weirdnet',
                      if (d.confirmShell) 'commands need a yes on the desktop',
                    ].join(' · '),
                  ),
                  trailing: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      IconButton(tooltip: 'Refresh', onPressed: () => c.refresh(d.id), icon: const Icon(Icons.refresh)),
                      IconButton(key: Key('forget-${d.id}'), tooltip: 'Unpair', onPressed: () => c.forget(d.id), icon: const Icon(Icons.link_off)),
                    ],
                  ),
                ),
            ],
          ),
        );
      },
    );
  }
}
