import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

// Version: 0.1.1 - CI trigger

import 'core/theme.dart';
import 'views/forest_view.dart';

void main() {
  runApp(const ProviderScope(child: ArborApp()));
}

/// Main application widget.
///
/// Sets up the custom theme and navigation structure.
/// This is intentionally simple - the magic happens in the graph views.
class ArborApp extends StatelessWidget {
  const ArborApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Arbor Visualizer',
      debugShowCheckedModeBanner: false,
      theme: ArborTheme.dark,
      home: const ForestView(),
    );
  }
}
