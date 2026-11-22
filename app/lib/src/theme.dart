// Bragi's look: the colour roles of the chosen theme (bragi-core's weirdnet-themes, shared with the
// desktop shell, crates/bragi-desktop/src/theme.rs, and the terminal app). Bragi's own dark
// palette until the core's palette arrives. `BragiColors` reads the palette in use; a theme change
// rebuilds the tree (see appearance.dart).
import 'package:material_ui/material_ui.dart';

import 'api.dart';

Color _hex(String h) => Color(0xFF000000 | int.parse(h.substring(1), radix: 16));

/// The twelve roles the widgets paint with.
class BragiPalette {
  const BragiPalette({
    required this.dark,
    required this.background,
    required this.surface,
    required this.surfaceVariant,
    required this.textPrimary,
    required this.textSecondary,
    required this.accent,
    required this.border,
    required this.selection,
    required this.error,
    required this.warm,
    required this.busy,
    required this.link,
  });

  /// Bragi's dark theme (brand/BRAND.md).
  static const bragiDark = BragiPalette(
    dark: true,
    background: Color(0xFF14161B),
    surface: Color(0xFF1C1F26),
    surfaceVariant: Color(0xFF2A2E37),
    textPrimary: Color(0xFFF4F1E8),
    textSecondary: Color(0xFFA9ABB3),
    accent: Color(0xFFF5B83D),
    border: Color(0xFF3A3F4B),
    selection: Color(0xFF5A4518),
    error: Color(0xFFFF5C70),
    warm: Color(0xFF4CD07D),
    busy: Color(0xFFFF9F43),
    link: Color(0xFF7FD4F5),
  );

  /// From the core's palette: the same mapping as the desktop shell.
  factory BragiPalette.fromView(PaletteView v) {
    final m = {for (final r in v.roles) r.role: _hex(r.hex)};
    Color c(String role) => m[role] ?? const Color(0xFF808080);
    return BragiPalette(
      dark: v.dark,
      background: c('bg'),
      surface: c('surface'),
      surfaceVariant: _hex(v.raised),
      textPrimary: c('fg'),
      textSecondary: c('muted'),
      accent: c('accent'),
      border: c('border'),
      selection: c('selection_bg'),
      error: c('error'),
      warm: c('success'),
      busy: c('warning'),
      link: c('link'),
    );
  }

  final bool dark;
  final Color background;
  final Color surface;
  final Color surfaceVariant;
  final Color textPrimary;
  final Color textSecondary;
  final Color accent;
  final Color border;
  final Color selection;
  final Color error;
  final Color warm;
  final Color busy;
  final Color link;
}

abstract final class BragiColors {
  static BragiPalette _p = BragiPalette.bragiDark;

  static BragiPalette get palette => _p;
  static void use(BragiPalette p) => _p = p;

  static Color get background => _p.background;
  static Color get surface => _p.surface;
  static Color get surfaceVariant => _p.surfaceVariant;
  static Color get textPrimary => _p.textPrimary;
  static Color get textSecondary => _p.textSecondary;
  static Color get accent => _p.accent;
  static Color get border => _p.border;
  static Color get selection => _p.selection;
  static Color get error => _p.error;
  static Color get warm => _p.warm;
  static Color get busy => _p.busy;

  /// Glacier in Bragi's own theme, shared with Ymir.
  static Color get link => _p.link;
}

ThemeData bragiTheme() {
  final p = BragiColors.palette;
  final base = p.dark ? const ColorScheme.dark() : const ColorScheme.light();
  final scheme = base.copyWith(
    primary: p.accent,
    onPrimary: p.background,
    surface: p.surface,
    onSurface: p.textPrimary,
    secondaryContainer: p.selection,
    onSecondaryContainer: p.textPrimary,
    surfaceContainerHighest: p.surfaceVariant,
    outline: p.border,
    error: p.error,
    tertiary: p.link,
  );
  return ThemeData(
    useMaterial3: true,
    colorScheme: scheme,
    scaffoldBackgroundColor: p.background,
    appBarTheme: AppBarTheme(backgroundColor: p.surface, foregroundColor: p.textPrimary),
    drawerTheme: DrawerThemeData(backgroundColor: p.surface),
  );
}
