// Linking this phone to the user's Weirdnet: show a code, compare the check code, done.
import 'package:material_ui/material_ui.dart';
import 'package:flutter/services.dart';
import 'package:qr/qr.dart';

import 'controller.dart';
import 'theme.dart';

class PairingScreen extends StatelessWidget {
  const PairingScreen({super.key, required this.controller});

  final BragiController controller;

  @override
  Widget build(BuildContext context) {
    final c = controller;
    final pair = c.state?.pair;
    final text = Theme.of(context).textTheme;
    final children = switch (c.pairState) {
      'offering' => [
          Text('Open this code on your other device', style: text.titleLarge, textAlign: TextAlign.center),
          const SizedBox(height: 16),
          QrView(data: pair!.link!, size: 240),
          const SizedBox(height: 12),
          SelectableText(pair.link!, style: text.bodySmall?.copyWith(color: BragiColors.textSecondary)),
          TextButton.icon(
            key: const Key('copy-code'),
            onPressed: () => Clipboard.setData(ClipboardData(text: pair.link!)),
            icon: const Icon(Icons.copy, size: 16),
            label: const Text('Copy'),
          ),
          Text('It works for two minutes.', style: text.bodySmall?.copyWith(color: BragiColors.textSecondary)),
        ],
      'confirm' => [
          Text('Do both screens show this code?', style: text.titleLarge, textAlign: TextAlign.center),
          const SizedBox(height: 16),
          Text(
            '${pair!.sas!.substring(0, 3)} ${pair.sas!.substring(3)}',
            key: const Key('sas'),
            style: text.displaySmall?.copyWith(color: BragiColors.accent),
          ),
          const SizedBox(height: 20),
          Wrap(spacing: 12, runSpacing: 8, alignment: WrapAlignment.center, children: [
            FilledButton(key: const Key('codes-match'), onPressed: () => c.confirmCode(true), child: const Text('They match')),
            OutlinedButton(key: const Key('codes-differ'), onPressed: () => c.confirmCode(false), child: const Text("They don't")),
          ]),
        ],
      'waiting' => [
          Text('Confirm on the other device', style: text.titleLarge),
          const SizedBox(height: 12),
          Text('Code ${pair!.sas}. Waiting for it…', style: text.bodyMedium?.copyWith(color: BragiColors.textSecondary)),
        ],
      _ => [
          Text('Link Bragi to your Weirdnet', style: text.titleLarge, textAlign: TextAlign.center),
          const SizedBox(height: 12),
          Text(
            'Show a code here, then open it on a device that is already linked: Morph, Ymir or another Bragi.',
            style: text.bodyMedium?.copyWith(color: BragiColors.textSecondary),
            textAlign: TextAlign.center,
          ),
          if (pair?.message != null) ...[
            const SizedBox(height: 12),
            Text(pair!.message!, style: text.bodyMedium?.copyWith(color: BragiColors.error)),
          ],
          const SizedBox(height: 20),
          FilledButton(key: const Key('show-code'), onPressed: c.showCode, child: const Text('Show a link code')),
        ],
    };
    return Scaffold(
      body: SafeArea(
        child: Center(
          child: SingleChildScrollView(
            padding: const EdgeInsets.all(20),
            child: Card(
              color: BragiColors.surface,
              child: Padding(
                padding: const EdgeInsets.all(24),
                child: Column(mainAxisSize: MainAxisSize.min, children: [
                  ...children,
                  if (c.notice != null) Text(c.notice!, style: const TextStyle(color: BragiColors.error)),
                ]),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// A QR code drawn black on white.
class QrView extends StatelessWidget {
  const QrView({super.key, required this.data, required this.size});

  final String data;
  final double size;

  @override
  Widget build(BuildContext context) {
    final code = QrCode(payload: QrPayload.fromString(data), errorCorrectLevel: QrErrorCorrectLevel.low);
    return Container(
      key: const Key('qr'),
      color: Colors.white,
      padding: const EdgeInsets.all(10),
      child: CustomPaint(size: Size.square(size), painter: _QrPainter(QrImage(code))),
    );
  }
}

class _QrPainter extends CustomPainter {
  _QrPainter(this.image);

  final QrImage image;

  @override
  void paint(Canvas canvas, Size size) {
    final cell = size.width / image.moduleCount;
    final paint = Paint()..color = Colors.black;
    for (var y = 0; y < image.moduleCount; y++) {
      for (var x = 0; x < image.moduleCount; x++) {
        if (image.isDark(y, x)) canvas.drawRect(Rect.fromLTWH(x * cell, y * cell, cell + 0.5, cell + 0.5), paint);
      }
    }
  }

  @override
  bool shouldRepaint(_QrPainter old) => old.image != image;
}
