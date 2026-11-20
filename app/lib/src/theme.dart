// Bragi's dark theme; the same colour roles as the desktop shell (crates/bragi-desktop/src/theme.rs).
import 'package:material_ui/material_ui.dart';

abstract final class BragiColors {
  static const background = Color(0xFF14161B);
  static const surface = Color(0xFF1C1F26);
  static const surfaceVariant = Color(0xFF2A2E37);
  static const textPrimary = Color(0xFFF4F1E8);
  static const textSecondary = Color(0xFFA9ABB3);
  static const accent = Color(0xFFF5B83D);
  static const border = Color(0xFF3A3F4B);
  static const selection = Color(0xFF5A4518);
  static const error = Color(0xFFFF5C70);
  static const warm = Color(0xFF4CD07D);
  static const busy = Color(0xFFFF9F43);
  /// Glacier, shared with Ymir.
  static const link = Color(0xFF7FD4F5);
}

ThemeData bragiTheme() {
  final scheme = const ColorScheme.dark(
    primary: BragiColors.accent,
    onPrimary: BragiColors.background,
    surface: BragiColors.surface,
    onSurface: BragiColors.textPrimary,
    secondaryContainer: BragiColors.selection,
    onSecondaryContainer: BragiColors.textPrimary,
    surfaceContainerHighest: BragiColors.surfaceVariant,
    outline: BragiColors.border,
    error: BragiColors.error,
    tertiary: BragiColors.link,
  );
  return ThemeData(
    useMaterial3: true,
    colorScheme: scheme,
    scaffoldBackgroundColor: BragiColors.background,
    appBarTheme: const AppBarTheme(backgroundColor: BragiColors.surface, foregroundColor: BragiColors.textPrimary),
    drawerTheme: const DrawerThemeData(backgroundColor: BragiColors.surface),
  );
}
