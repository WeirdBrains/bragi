// The Theme screens (LF-33): a picker with a live preview of the whole app (tap a theme to see
// it, Use to keep it), and an editor with a colour per role, a hex box, the theme's palette and
// HSL sliders. Duplicate, rename, reset, delete, import and export are in the menu. Every action is
// one core call through the appearance controller; the colours are weirdnet-themes' TOML, shared
// with the desktop and terminal apps.
import 'package:flutter/services.dart';
import 'package:material_ui/material_ui.dart';

import 'api.dart';
import 'appearance.dart';
import 'theme.dart';

Color _color(String hex) => Color(0xFF000000 | int.parse(hex.substring(1), radix: 16));

String _toHex(Color c) {
  String two(double v) => (v * 255).round().clamp(0, 255).toRadixString(16).padLeft(2, '0');
  return '#${two(c.r)}${two(c.g)}${two(c.b)}';
}

/// `#abc` / `#aabbcc` (`#` optional) as `#rrggbb`, or null.
String? parseHex(String s) {
  var h = s.trim();
  if (h.startsWith('#')) h = h.substring(1);
  if (!RegExp(r'^[0-9a-fA-F]+$').hasMatch(h)) return null;
  if (h.length == 3) h = h.split('').map((c) => '$c$c').join();
  if (h.length != 6) return null;
  return '#${h.toLowerCase()}';
}

class ThemeScreen extends StatefulWidget {
  const ThemeScreen({super.key, required this.appearance});

  final AppearanceController appearance;

  @override
  State<ThemeScreen> createState() => _ThemeScreenState();
}

class _ThemeScreenState extends State<ThemeScreen> {
  late String sel = widget.appearance.pref.theme;
  late String mode = widget.appearance.pref.mode;
  bool saved = true;

  AppearanceController get a => widget.appearance;

  @override
  void initState() {
    super.initState();
    a.reload();
  }

  @override
  void dispose() {
    // Leaving without Use puts the saved look back.
    if (a.pref.theme != sel || a.pref.mode != mode) {
      WidgetsBinding.instance.addPostFrameCallback((_) => a.restore());
    }
    super.dispose();
  }

  void _pick(String name) {
    setState(() => sel = name);
    a.preview(name, mode);
  }

  void _mode(String m) {
    setState(() => mode = m);
    a.preview(sel, m);
  }

  Future<void> _use() async {
    final ok = await a.use(sel, mode);
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(ok ? 'Using $sel ($mode).' : a.error ?? 'Could not save.')));
  }

  Future<String?> _ask(String title, String initial, {String hint = '', bool multiline = false}) {
    final c = TextEditingController(text: initial);
    return showDialog<String>(
      context: context,
      builder: (ctx) => AlertDialog(
        title: Text(title),
        content: TextField(
          key: const Key('theme-ask'),
          controller: c,
          autofocus: true,
          minLines: multiline ? 6 : 1,
          maxLines: multiline ? 12 : 1,
          decoration: InputDecoration(hintText: hint),
        ),
        actions: [
          TextButton(onPressed: () => Navigator.pop(ctx), child: const Text('Cancel')),
          TextButton(key: const Key('theme-ask-ok'), onPressed: () => Navigator.pop(ctx, c.text.trim()), child: const Text('OK')),
        ],
      ),
    );
  }

  void _say(String text) => ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(text)));

  Future<void> _menu(ThemeRowView r, String what) async {
    switch (what) {
      case 'edit':
        final name = await a.run(() => a.api.themeEditable(r.name));
        if (name == null || !mounted) return _say(a.error ?? '');
        setState(() => sel = name);
        await Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => ThemeEditorScreen(appearance: a, name: name, mode: mode)));
        if (mounted) a.preview(sel, mode);
      case 'duplicate':
        final n = await _ask('Name the copy', '${r.name}-copy');
        if (n == null || n.isEmpty) return;
        final made = await a.run(() => a.api.themeDuplicate(r.name, n));
        if (made != null) _pick(made);
        if (made == null && mounted) _say(a.error ?? '');
      case 'rename':
        final n = await _ask('New name', r.name);
        if (n == null || n.isEmpty) return;
        final ok = await a.run(() => a.api.themeRename(r.name, n).then((_) => true));
        if (ok == true) {
          await a.load();
          _pick(n);
        } else if (mounted) {
          _say(a.error ?? '');
        }
      case 'reset':
        final ok = await a.run(() => a.api.themeDelete(r.name).then((_) => true));
        if (ok == true) _pick(r.name);
        if (mounted) _say(ok == true ? '${r.name} is as it ships.' : a.error ?? '');
      case 'delete':
        final yes = await showDialog<bool>(
          context: context,
          builder: (ctx) => AlertDialog(
            title: Text('Delete ${r.name}?'),
            content: const Text('Its file goes from this device.'),
            actions: [
              TextButton(onPressed: () => Navigator.pop(ctx, false), child: const Text('Keep')),
              TextButton(key: const Key('theme-delete-yes'), onPressed: () => Navigator.pop(ctx, true), child: const Text('Delete')),
            ],
          ),
        );
        if (yes != true) return;
        final ok = await a.run(() => a.api.themeDelete(r.name).then((_) => true));
        if (ok == true) {
          await a.load();
          _pick(a.pref.theme);
        } else if (mounted) {
          _say(a.error ?? '');
        }
      case 'export':
        final text = await a.run(() => a.api.themeExportText(r.name));
        if (text == null) return;
        await Clipboard.setData(ClipboardData(text: text));
        if (mounted) _say('${r.name}.toml copied to the clipboard.');
    }
  }

  Future<void> _import() async {
    final text = await _ask('Paste a theme file (TOML)', '', hint: '[meta]\nname = "…"', multiline: true);
    if (text == null || text.isEmpty) return;
    final n = await a.run(() => a.api.themeImportText(text, null));
    if (!mounted) return;
    if (n == null) return _say(a.error ?? '');
    _pick(n);
    _say('Imported $n.');
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: a,
      builder: (context, _) {
        final dirty = a.pref.theme != sel || a.pref.mode != mode;
        return Scaffold(
          appBar: AppBar(
            title: const Text('Theme'),
            actions: [
              IconButton(key: const Key('theme-import'), tooltip: 'Import', icon: const Icon(Icons.file_download_outlined), onPressed: _import),
              TextButton(key: const Key('theme-use'), onPressed: dirty ? _use : null, child: const Text('Use')),
            ],
          ),
          body: ListView(
            key: const Key('theme-list'),
            children: [
              Padding(
                padding: const EdgeInsets.all(16),
                child: SegmentedButton<String>(
                  key: const Key('theme-mode'),
                  segments: const [
                    ButtonSegment(value: 'dark', label: Text('Dark')),
                    ButtonSegment(value: 'light', label: Text('Light')),
                    ButtonSegment(value: 'system', label: Text('System')),
                  ],
                  selected: {mode},
                  onSelectionChanged: (s) => _mode(s.first),
                ),
              ),
              if (a.error != null) Padding(padding: const EdgeInsets.symmetric(horizontal: 16), child: Text(a.error!, style: TextStyle(color: BragiColors.error))),
              for (final r in a.rows) _row(r),
              const _Preview(),
            ],
          ),
        );
      },
    );
  }

  Widget _row(ThemeRowView r) {
    final p = r.error == null ? a.api.themePalette(r.name, a.darkFor(mode)) : null;
    final on = r.name == sel;
    final tag = r.error != null
        ? 'broken'
        : r.builtin
        ? (r.userFile ? 'edited' : 'built-in')
        : 'yours';
    return ListTile(
      key: Key('theme-${r.name}'),
      selected: on,
      selectedTileColor: BragiColors.selection,
      leading: Icon(r.name == a.pref.theme ? Icons.radio_button_checked : Icons.radio_button_off, color: r.name == a.pref.theme ? BragiColors.warm : BragiColors.textSecondary),
      title: Text(r.displayName),
      subtitle: Text(r.error ?? tag, style: TextStyle(color: r.error != null ? BragiColors.error : BragiColors.textSecondary, fontSize: 12)),
      trailing: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (p != null)
            for (final role in const ['bg', 'accent', 'fg', 'up'])
              Container(
                margin: const EdgeInsets.only(left: 3),
                width: 12,
                height: 12,
                decoration: BoxDecoration(
                  color: _color(p.roles.firstWhere((x) => x.role == role).hex),
                  shape: BoxShape.circle,
                  border: Border.all(color: BragiColors.border),
                ),
              ),
          PopupMenuButton<String>(
            key: Key('theme-menu-${r.name}'),
            onSelected: (w) => _menu(r, w),
            itemBuilder: (_) => [
              const PopupMenuItem(value: 'edit', child: Text('Edit colours')),
              const PopupMenuItem(value: 'duplicate', child: Text('Duplicate')),
              if (!r.builtin) const PopupMenuItem(value: 'rename', child: Text('Rename')),
              if (r.builtin && r.userFile) const PopupMenuItem(value: 'reset', child: Text('Reset to the original')),
              if (!r.builtin) const PopupMenuItem(value: 'delete', child: Text('Delete')),
              const PopupMenuItem(value: 'export', child: Text('Export (copy)')),
            ],
          ),
        ],
      ),
      onTap: () => _pick(r.name),
    );
  }
}

/// A sample of the app in the colours being shown.
class _Preview extends StatelessWidget {
  const _Preview();

  @override
  Widget build(BuildContext context) {
    return Container(
      key: const Key('theme-preview'),
      margin: const EdgeInsets.all(16),
      padding: const EdgeInsets.all(14),
      decoration: BoxDecoration(color: BragiColors.surface, borderRadius: BorderRadius.circular(10), border: Border.all(color: BragiColors.border)),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('PREVIEW', style: TextStyle(fontSize: 11, color: BragiColors.textSecondary)),
          const SizedBox(height: 8),
          Align(
            alignment: Alignment.centerRight,
            child: Container(
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
              decoration: BoxDecoration(color: BragiColors.selection, borderRadius: BorderRadius.circular(10)),
              child: Text('How do I undo a rebase?', style: TextStyle(color: BragiColors.textPrimary)),
            ),
          ),
          const SizedBox(height: 8),
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            decoration: BoxDecoration(color: BragiColors.surfaceVariant, borderRadius: BorderRadius.circular(10)),
            child: Text('Use git reflog to find where the branch was, then reset to it.', style: TextStyle(color: BragiColors.textPrimary)),
          ),
          const SizedBox(height: 8),
          Text('secondary text · a hint', style: TextStyle(color: BragiColors.textSecondary, fontSize: 12)),
          Text('a link', style: TextStyle(color: BragiColors.link, decoration: TextDecoration.underline)),
          const SizedBox(height: 6),
          Row(
            children: [
              Text('✓ done   ', style: TextStyle(color: BragiColors.warm)),
              Text('! needs a look   ', style: TextStyle(color: BragiColors.busy)),
              Text('✗ error', style: TextStyle(color: BragiColors.error)),
            ],
          ),
          const SizedBox(height: 10),
          FilledButton(onPressed: () {}, child: const Text('Primary')),
        ],
      ),
    );
  }
}

/// Edits the roles of one theme: a swatch and colour per role; tap one for the picker. Edits are
/// live in the whole app and saved with Save.
class ThemeEditorScreen extends StatefulWidget {
  const ThemeEditorScreen({super.key, required this.appearance, required this.name, required this.mode});

  final AppearanceController appearance;
  final String name;
  final String mode;

  @override
  State<ThemeEditorScreen> createState() => _ThemeEditorScreenState();
}

class _ThemeEditorScreenState extends State<ThemeEditorScreen> {
  late bool dark = widget.appearance.darkFor(widget.mode);

  /// The saved palette per variant and the pending roles on top of it.
  final Map<bool, PaletteView> _saved = {};
  final Map<bool, Map<String, String>> _pending = {true: {}, false: {}};
  late final List<RoleInfoView> roles = widget.appearance.api.themeRoles();
  String? msg;

  AppearanceController get a => widget.appearance;

  PaletteView _base(bool d) => _saved.putIfAbsent(d, () => a.api.themePalette(widget.name, d));

  @override
  void initState() {
    super.initState();
    // After this first frame: the preview rebuilds the app, which is mid-build now.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _show();
    });
  }

  @override
  void dispose() {
    WidgetsBinding.instance.addPostFrameCallback((_) => a.restore());
    super.dispose();
  }

  String _hex(String role) => _pending[dark]![role] ?? _base(dark).roles.firstWhere((r) => r.role == role).hex;

  bool get dirty => _pending[true]!.isNotEmpty || _pending[false]!.isNotEmpty;

  /// The whole app in what is being edited.
  void _show() {
    final b = _base(dark);
    a.previewPalette(
      PaletteView(
        theme: b.theme,
        dark: dark,
        raised: b.raised,
        roles: [for (final r in b.roles) RoleColorView(role: r.role, hex: _pending[dark]![r.role] ?? r.hex)],
      ),
    );
  }

  void _set(String role, String hex) {
    setState(() => _pending[dark]![role] = hex);
    _show();
  }

  void _undo(String role) {
    setState(() => _pending[dark]!.remove(role));
    _show();
  }

  Future<void> _save() async {
    try {
      for (final d in [true, false]) {
        final p = _pending[d]!;
        if (p.isEmpty) continue;
        final v = await a.api.themeApply(widget.name, d, [for (final e in p.entries) RoleEdit(role: e.key, value: e.value)]);
        _saved[d] = v;
        p.clear();
      }
      final warn = await a.api.themeWarnings(widget.name, dark);
      await a.load();
      setState(() => msg = warn.isEmpty ? 'Saved ${widget.name}.' : 'Saved ${widget.name}. Check: ${warn.first}');
    } catch (e) {
      setState(() => msg = '$e');
    }
  }

  Future<void> _pickColor(RoleInfoView r) async {
    await showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      builder: (ctx) => _ColorSheet(
        role: r,
        hex: _hex(r.role),
        original: _base(dark).roles.firstWhere((x) => x.role == r.role).hex,
        swatches: {for (final x in _base(dark).roles) x.hex}.toList(),
        onChanged: (h) => _set(r.role, h),
        onUndo: () => _undo(r.role),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: Text(widget.name),
        actions: [
          TextButton(key: const Key('edit-variant'), onPressed: () => setState(() {
            dark = !dark;
            _show();
          }), child: Text(dark ? 'Edit light' : 'Edit dark')),
          TextButton(key: const Key('edit-save'), onPressed: dirty ? _save : null, child: const Text('Save')),
        ],
      ),
      // The save message stays in view however far the list is scrolled.
      bottomNavigationBar: msg == null
          ? null
          : SafeArea(child: Padding(padding: const EdgeInsets.all(16), child: Text(msg!, key: const Key('edit-msg'), style: TextStyle(color: BragiColors.textSecondary)))),
      body: ListView(
        key: const Key('role-list'),
        children: [
          for (final r in roles)
            ListTile(
              key: Key('role-${r.role}'),
              dense: true,
              leading: Container(
                width: 28,
                height: 28,
                decoration: BoxDecoration(color: _color(_hex(r.role)), borderRadius: BorderRadius.circular(6), border: Border.all(color: BragiColors.border)),
              ),
              title: Text('${r.role}${_pending[dark]!.containsKey(r.role) ? ' •' : ''}'),
              subtitle: Text(r.label, style: TextStyle(color: BragiColors.textSecondary, fontSize: 12)),
              trailing: Text(_hex(r.role), style: TextStyle(color: BragiColors.textSecondary, fontFamily: 'monospace')),
              onTap: () => _pickColor(r),
            ),
          const _Preview(),
        ],
      ),
    );
  }
}

/// The picker for one role: a hex box, the theme's colours, and hue, saturation and lightness.
class _ColorSheet extends StatefulWidget {
  const _ColorSheet({required this.role, required this.hex, required this.original, required this.swatches, required this.onChanged, required this.onUndo});

  final RoleInfoView role;
  final String hex;
  final String original;
  final List<String> swatches;
  final ValueChanged<String> onChanged;
  final VoidCallback onUndo;

  @override
  State<_ColorSheet> createState() => _ColorSheetState();
}

class _ColorSheetState extends State<_ColorSheet> {
  late HSLColor hsl = HSLColor.fromColor(_color(widget.hex));
  late final TextEditingController text = TextEditingController(text: widget.hex);
  bool bad = false;

  void _apply(HSLColor c) {
    setState(() {
      hsl = c;
      final h = _toHex(c.toColor());
      text.text = h;
      bad = false;
    });
    widget.onChanged(_toHex(c.toColor()));
  }

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: EdgeInsets.fromLTRB(16, 16, 16, 16 + MediaQuery.of(context).viewInsets.bottom),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('${widget.role.role} — ${widget.role.label}'),
          const SizedBox(height: 10),
          Row(
            children: [
              Container(width: 36, height: 36, decoration: BoxDecoration(color: hsl.toColor(), borderRadius: BorderRadius.circular(6), border: Border.all(color: BragiColors.border))),
              const SizedBox(width: 12),
              SizedBox(
                width: 130,
                child: TextField(
                  key: const Key('hex-field'),
                  controller: text,
                  decoration: InputDecoration(isDense: true, errorText: bad ? '#rgb or #rrggbb' : null),
                  onChanged: (v) {
                    final h = parseHex(v);
                    if (h == null) return setState(() => bad = true);
                    setState(() {
                      bad = false;
                      hsl = HSLColor.fromColor(_color(h));
                    });
                    widget.onChanged(h);
                  },
                ),
              ),
              const Spacer(),
              TextButton(
                key: const Key('role-undo'),
                onPressed: () {
                  widget.onUndo();
                  _apply(HSLColor.fromColor(_color(widget.original)));
                },
                child: const Text('Undo'),
              ),
            ],
          ),
          const SizedBox(height: 10),
          Wrap(
            spacing: 6,
            runSpacing: 6,
            children: [
              for (final h in widget.swatches)
                GestureDetector(
                  key: Key('swatch-$h'),
                  onTap: () => _apply(HSLColor.fromColor(_color(h))),
                  child: Container(width: 26, height: 26, decoration: BoxDecoration(color: _color(h), borderRadius: BorderRadius.circular(5), border: Border.all(color: BragiColors.border))),
                ),
            ],
          ),
          Slider(key: const Key('hue'), value: hsl.hue, max: 359.9, onChanged: (v) => _apply(hsl.withHue(v))),
          Slider(key: const Key('sat'), value: hsl.saturation, onChanged: (v) => _apply(hsl.withSaturation(v))),
          Slider(key: const Key('light'), value: hsl.lightness, onChanged: (v) => _apply(hsl.withLightness(v))),
        ],
      ),
    );
  }
}
