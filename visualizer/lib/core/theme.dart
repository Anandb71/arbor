import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';

/// Bespoke dark theme for the Arbor Visualizer.
///
/// We deliberately avoid Material defaults to create a cinematic,
/// high-contrast look that makes code graphs feel alive.
class ArborTheme {
  ArborTheme._();

  // Core palette - designed for depth and clarity
  static const Color background = Color(0xFF0A0A0F);
  static const Color surface = Color(0xFF12121A);
  static const Color surfaceLight = Color(0xFF1A1A25);
  
  // Accent colors for different node types
  static const Color function = Color(0xFF00D9FF);    // Electric cyan
  static const Color classType = Color(0xFF9D4EDD);   // Royal purple
  static const Color method = Color(0xFF00F5A0);      // Mint green
  static const Color variable = Color(0xFFFFB800);    // Warm amber
  static const Color keyword = Color(0xFFF72585);     // Neon Pink
  static const Color importType = Color(0xFFFF6B6B); // Coral red
  
  // UI elements
  static const Color textPrimary = Color(0xFFE8E8E8);
  static const Color textSecondary = Color(0xFF8888A0);
  static const Color textMuted = Color(0xFF555566);
  static const Color border = Color(0xFF2A2A35);
  static const Color glow = Color(0xFF00D9FF);

  /// Returns the color for a given node kind.
  static Color colorForKind(String kind) {
    switch (kind.toLowerCase()) {
      case 'function':
        return function;
      case 'class':
        return classType;
      case 'method':
        return method;
      case 'variable':
      case 'constant':
        return variable;
      case 'import':
        return importType;
      default:
        return textSecondary;
    }
  }

  /// The dark theme data.
  static ThemeData get dark {
    return ThemeData.dark().copyWith(
      scaffoldBackgroundColor: background,
      colorScheme: const ColorScheme.dark(
        primary: function,
        secondary: classType,
        surface: surface,
        error: importType,
        onPrimary: background,
        onSecondary: background,
        onSurface: textPrimary,
        onError: background,
      ),
      textTheme: GoogleFonts.jetBrainsMonoTextTheme(
        ThemeData.dark().textTheme,
      ).copyWith(
        displayLarge: GoogleFonts.outfit(
          fontSize: 48,
          fontWeight: FontWeight.w700,
          color: textPrimary,
          letterSpacing: -1,
        ),
        displayMedium: GoogleFonts.outfit(
          fontSize: 32,
          fontWeight: FontWeight.w600,
          color: textPrimary,
        ),
        titleLarge: GoogleFonts.outfit(
          fontSize: 20,
          fontWeight: FontWeight.w600,
          color: textPrimary,
        ),
        bodyLarge: GoogleFonts.jetBrainsMono(
          fontSize: 14,
          color: textPrimary,
        ),
        bodyMedium: GoogleFonts.jetBrainsMono(
          fontSize: 12,
          color: textSecondary,
        ),
        labelSmall: GoogleFonts.jetBrainsMono(
          fontSize: 10,
          color: textMuted,
          letterSpacing: 0.5,
        ),
      ),
      /*
      cardTheme: CardTheme(
        color: surface,
        elevation: 0,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(12),
          side: const BorderSide(color: border, width: 1),
        ),
      ),
      */
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        fillColor: surface,
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(8),
          borderSide: const BorderSide(color: border),
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(8),
          borderSide: const BorderSide(color: border),
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(8),
          borderSide: const BorderSide(color: function, width: 2),
        ),
        hintStyle: GoogleFonts.jetBrainsMono(
          color: textMuted,
          fontSize: 14,
        ),
      ),
    );
  }
}
