// Draws a reply's Markdown. bragi-core parses it (the same tree the desktop and the terminal UI
// draw) and hands it over as JSON; this only maps blocks to widgets. Links open in the system
// browser, code blocks have their language and a Copy button. Nothing is fetched, and raw HTML
// arrives as plain text.
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';
import 'package:url_launcher/url_launcher.dart';

import 'theme.dart';

typedef Json = Map<String, dynamic>;

/// Opens [url] (already checked by the core to be http, https or mailto) outside the app.
Future<void> openLink(String url) async {
  final uri = Uri.tryParse(url);
  if (uri != null) await launchUrl(uri, mode: LaunchMode.externalApplication);
}

const _mono = 'monospace';
const _monoFallback = ['Menlo', 'Consolas', 'DejaVu Sans Mono', 'Liberation Mono', 'Roboto Mono'];

Color _tokenColor(String kind) => switch (kind) {
      'keyword' => const Color(0xFFC792EA),
      'literal' || 'number' => const Color(0xFFF78C6C),
      'string' => const Color(0xFFC3E88D),
      'comment' => const Color(0xFF7A7F8C),
      'type' => BragiColors.accent,
      'function' => BragiColors.link,
      'added' => BragiColors.warm,
      'removed' => BragiColors.error,
      _ => BragiColors.textPrimary,
    };

class MarkdownView extends StatefulWidget {
  const MarkdownView({super.key, required this.doc, this.onOpen = openLink});

  /// `bragi_core::markdown::Document` as JSON.
  final Json doc;
  final Future<void> Function(String url) onOpen;

  @override
  State<MarkdownView> createState() => _MarkdownViewState();
}

class _MarkdownViewState extends State<MarkdownView> {
  final _recognizers = <TapGestureRecognizer>[];
  String? _hover;

  void _clearRecognizers() {
    for (final r in _recognizers) {
      r.dispose();
    }
    _recognizers.clear();
  }

  @override
  void dispose() {
    _clearRecognizers();
    super.dispose();
  }

  static TextStyle get _base => TextStyle(fontSize: 15, height: 1.4, color: BragiColors.textPrimary);

  TextSpan _spans(List<dynamic> spans, TextStyle style) {
    return TextSpan(
      style: style,
      children: [
        for (final s in spans.cast<Json>()) _span(s, style),
      ],
    );
  }

  InlineSpan _span(Json s, TextStyle base) {
    final link = s['link'] as String?;
    var style = base.copyWith(
      fontWeight: s['bold'] == true ? FontWeight.bold : null,
      fontStyle: s['italic'] == true ? FontStyle.italic : null,
      decoration: link != null ? TextDecoration.underline : (s['strike'] == true ? TextDecoration.lineThrough : null),
    );
    if (s['code'] == true) {
      style = style.copyWith(fontFamily: _mono, fontFamilyFallback: _monoFallback, backgroundColor: BragiColors.surfaceVariant, fontSize: (base.fontSize ?? 15) - 1);
    }
    if (link == null) return TextSpan(text: s['text'] as String, style: style);
    final r = TapGestureRecognizer()..onTap = () => widget.onOpen(link);
    _recognizers.add(r);
    return TextSpan(
      text: s['text'] as String,
      style: style.copyWith(color: BragiColors.link, decorationColor: BragiColors.link),
      recognizer: r,
      mouseCursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hover = link),
      onExit: (_) => setState(() => _hover = null),
    );
  }

  Widget _block(Json b, {bool muted = false}) {
    final base = muted ? _base.copyWith(color: BragiColors.textSecondary) : _base;
    switch (b['type']) {
      case 'paragraph':
        return Text.rich(_spans(b['spans'] as List, base));
      case 'heading':
        final level = b['level'] as int;
        final size = switch (level) { 1 => 22.0, 2 => 19.0, 3 => 17.0, _ => 15.0 };
        return Padding(
          padding: const EdgeInsets.only(top: 4),
          child: Text.rich(_spans(b['spans'] as List, base.copyWith(fontSize: size, fontWeight: FontWeight.bold, height: 1.3))),
        );
      case 'quote':
        return Container(
          padding: const EdgeInsets.only(left: 10),
          decoration: BoxDecoration(border: Border(left: BorderSide(color: BragiColors.border, width: 3))),
          child: _column(b['blocks'] as List, muted: true),
        );
      case 'list':
        final start = b['start'] as int?;
        final items = (b['items'] as List).cast<Json>();
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            for (final (n, item) in items.indexed)
              Padding(
                padding: const EdgeInsets.only(bottom: 3),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    SizedBox(
                      width: 24,
                      child: switch (item['checked']) {
                        true => Padding(padding: EdgeInsets.only(top: 3), child: Icon(Icons.check_box, size: 16, color: BragiColors.accent)),
                        false => Padding(padding: EdgeInsets.only(top: 3), child: Icon(Icons.check_box_outline_blank, size: 16, color: BragiColors.textSecondary)),
                        _ => Text(start != null ? '${start + n}.' : '•', style: base.copyWith(color: BragiColors.textSecondary)),
                      },
                    ),
                    Expanded(child: _column(item['blocks'] as List, muted: muted)),
                  ],
                ),
              ),
          ],
        );
      case 'code':
        return _Code(lang: b['lang'] as String?, code: b['code'] as String, tokens: (b['tokens'] as List).cast<Json>());
      case 'table':
        final align = (b['align'] as List).cast<String>();
        final head = (b['head'] as List);
        final rows = (b['rows'] as List).cast<List>();
        TextAlign ta(int c) => switch (c < align.length ? align[c] : 'none') { 'center' => TextAlign.center, 'right' => TextAlign.right, _ => TextAlign.left };
        Widget cell(List spans, int c, {bool bold = false}) => Padding(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 5),
              child: Text.rich(_spans(spans, bold ? base.copyWith(fontWeight: FontWeight.bold) : base), textAlign: ta(c)),
            );
        return SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          child: Table(
            defaultColumnWidth: const IntrinsicColumnWidth(),
            border: TableBorder.all(color: BragiColors.border, borderRadius: BorderRadius.circular(6)),
            children: [
              TableRow(
                decoration: BoxDecoration(color: BragiColors.surfaceVariant),
                children: [for (final (c, h) in head.indexed) cell(h as List, c, bold: true)],
              ),
              for (final r in rows)
                TableRow(children: [for (var c = 0; c < head.length; c++) cell(c < r.length ? r[c] as List : const [], c)]),
            ],
          ),
        );
      case 'rule':
        return Divider(color: BragiColors.border, height: 12);
      default:
        return const SizedBox.shrink();
    }
  }

  Widget _column(List blocks, {bool muted = false}) => Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          for (final (i, b) in blocks.cast<Json>().indexed) ...[
            if (i > 0) const SizedBox(height: 8),
            _block(b, muted: muted),
          ],
        ],
      );

  @override
  Widget build(BuildContext context) {
    _clearRecognizers();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _column(widget.doc['blocks'] as List? ?? const []),
        if (_hover != null)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Text(_hover!, key: const Key('link-hover'), style: TextStyle(fontSize: 11, color: BragiColors.textSecondary), overflow: TextOverflow.ellipsis),
          ),
      ],
    );
  }
}

class _Code extends StatefulWidget {
  const _Code({required this.lang, required this.code, required this.tokens});

  final String? lang;
  final String code;
  final List<Json> tokens;

  @override
  State<_Code> createState() => _CodeState();
}

class _CodeState extends State<_Code> {
  bool _copied = false;

  Future<void> _copy() async {
    await Clipboard.setData(ClipboardData(text: widget.code));
    if (!mounted) return;
    setState(() => _copied = true);
    await Future<void>.delayed(const Duration(seconds: 2));
    if (mounted) setState(() => _copied = false);
  }

  @override
  Widget build(BuildContext context) {
    final style = TextStyle(fontFamily: _mono, fontFamilyFallback: _monoFallback, fontSize: 13, height: 1.45, color: BragiColors.textPrimary);
    return Container(
      decoration: BoxDecoration(
        color: BragiColors.background,
        border: Border.all(color: BragiColors.border),
        borderRadius: BorderRadius.circular(8),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Container(
            padding: const EdgeInsets.only(left: 10),
            decoration: BoxDecoration(border: Border(bottom: BorderSide(color: BragiColors.border))),
            child: Row(
              children: [
                Expanded(child: Text(widget.lang ?? 'text', style: TextStyle(fontSize: 11, color: BragiColors.textSecondary))),
                TextButton(
                  key: const Key('code-copy'),
                  onPressed: _copy,
                  style: TextButton.styleFrom(visualDensity: VisualDensity.compact, textStyle: const TextStyle(fontSize: 11)),
                  child: Text(_copied ? 'Copied' : 'Copy'),
                ),
              ],
            ),
          ),
          SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            padding: const EdgeInsets.all(10),
            child: Text.rich(
              TextSpan(
                style: style,
                children: [
                  for (final t in widget.tokens) TextSpan(text: t['text'] as String, style: TextStyle(color: _tokenColor(t['kind'] as String))),
                ],
              ),
              softWrap: false,
            ),
          ),
        ],
      ),
    );
  }
}
