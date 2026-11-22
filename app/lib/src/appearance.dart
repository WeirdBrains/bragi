// The look as the app keeps it: the chosen theme and mode, the palette in use, and the live
// preview the Theme screens drive. Every action is one core call; the palette is the core's.
import 'package:flutter/widgets.dart';

import 'api.dart';
import 'theme.dart';

/// Marks every widget for a rebuild, as a hot reload does: the colours are read from
/// [BragiColors] while building, so a theme change has to reach widgets that didn't change.
void rebuildAll() {
  void visit(Element e) {
    e.markNeedsBuild();
    e.visitChildren(visit);
  }

  WidgetsBinding.instance.rootElement?.visitChildren(visit);
}

class AppearanceController extends ChangeNotifier {
  AppearanceController(this.api);

  final BragiApi api;
  List<ThemeRowView> rows = const [];
  ThemePrefView pref = ThemePrefView(theme: 'bragi', mode: 'system');

  /// The platform's brightness, for system mode (the core can't ask a phone).
  bool? systemDark;
  String? error;

  /// The palette shown now: the saved look, or a preview of one.
  BragiPalette shown = BragiPalette.bragiDark;

  Future<void> load() async {
    try {
      pref = api.themePreference();
      rows = await api.themes();
      _show(BragiPalette.fromView(api.currentPalette(systemDark)));
    } catch (e) {
      error = '$e';
    }
    notifyListeners();
  }

  void _show(BragiPalette p) {
    shown = p;
    BragiColors.use(p);
  }

  /// The platform changed between light and dark.
  void platformBrightness(bool dark) {
    systemDark = dark;
    if (pref.mode == 'system') {
      _show(BragiPalette.fromView(api.currentPalette(systemDark)));
      notifyListeners();
      WidgetsBinding.instance.addPostFrameCallback((_) => rebuildAll());
    }
  }

  /// Whether the variant of mode [mode] is dark.
  bool darkFor(String mode) => switch (mode) {
    'dark' => true,
    'light' => false,
    _ => systemDark ?? true,
  };

  /// Shows [name] in [mode] without saving it.
  void preview(String name, String mode) {
    try {
      _show(BragiPalette.fromView(api.themePalette(name, darkFor(mode))));
      error = null;
    } catch (e) {
      error = '$e';
    }
    notifyListeners();
    WidgetsBinding.instance.addPostFrameCallback((_) => rebuildAll());
  }

  /// Shows a palette that is being edited and isn't saved.
  void previewPalette(PaletteView v) {
    _show(BragiPalette.fromView(v));
    notifyListeners();
    WidgetsBinding.instance.addPostFrameCallback((_) => rebuildAll());
  }

  /// Back to the saved look.
  void restore() {
    _show(BragiPalette.fromView(api.currentPalette(systemDark)));
    notifyListeners();
    WidgetsBinding.instance.addPostFrameCallback((_) => rebuildAll());
  }

  /// Saves the choice.
  Future<bool> use(String name, String mode) async {
    try {
      pref = await api.setThemePreference(name, mode);
      error = null;
      restore();
      return true;
    } catch (e) {
      error = '$e';
      notifyListeners();
      return false;
    }
  }

  Future<void> reload() async {
    rows = await api.themes();
    notifyListeners();
  }

  /// Runs a core action; its error text is [error].
  Future<T?> run<T>(Future<T> Function() f) async {
    try {
      final r = await f();
      error = null;
      await reload();
      return r;
    } catch (e) {
      error = '$e';
      notifyListeners();
      return null;
    }
  }
}
