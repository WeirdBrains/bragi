// Bragi's dark theme; the same colour roles as the desktop shell (crates/bragi-desktop/src/theme.rs).
import 'package:material_ui/material_ui.dart';

abstract final class BragiColors {
  static const background = Color(0xFF0E0F14);
  static const surface = Color(0xFF16171F);
  static const surfaceVariant = Color(0xFF22232E);
  static const textPrimary = Color(0xFFF3F3F8);
  static const textSecondary = Color(0xFFA6A8B8);
  static const accent = Color(0xFFF2B544);
  static const border = Color(0xFF3A3C4C);
  static const selection = Color(0xFF5A4518);
  static const error = Color(0xFFFF5C70);
  static const warm = Color(0xFF4CD07D);
  static const busy = Color(0xFFFF9F43);
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
  );
  return ThemeData(
    useMaterial3: true,
    colorScheme: scheme,
    scaffoldBackgroundColor: BragiColors.background,
    appBarTheme: const AppBarTheme(backgroundColor: BragiColors.surface, foregroundColor: BragiColors.textPrimary),
    drawerTheme: const DrawerThemeData(backgroundColor: BragiColors.surface),
  );
}
