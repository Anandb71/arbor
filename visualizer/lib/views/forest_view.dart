import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../core/providers.dart';
import '../core/theme.dart';
import '../graph/graph_widget.dart';

/// The main visualization view - the Logic Forest.
///
/// Wraps the interactive GraphWidget with UI overlays like the Inspector,
/// Search Bar, and Status Bar.
class ForestView extends ConsumerStatefulWidget {
  const ForestView({super.key});

  @override
  ConsumerState<ForestView> createState() => _ForestViewState();
}

class _ForestViewState extends ConsumerState<ForestView> {
  // Search
  final _searchController = TextEditingController();

  @override
  void dispose() {
    _searchController.dispose();
    super.dispose();
  }

  void _handleSearch(String query) {
    if (query.isEmpty) return;
    ref.read(graphProvider.notifier).search(query);
  }

  @override
  Widget build(BuildContext context) {
    final state = ref.watch(graphProvider);

    return Scaffold(
      backgroundColor: ArborTheme.background,
      body: Stack(
        children: [
          // Background gradient for depth
          _buildBackground(),

          // The Interactive Graph
          const GraphWidget(),

          // Top bar
          _buildTopBar(state),

          // Node inspector (right panel)
          if (state.selectedNodeId != null) _buildInspector(state),

          // Status bar
          _buildStatusBar(state),

          // Settings Panel
          _buildSettingsPanel(state),

          // Loading overlay
          if (state.isLoading)
            Container(
              color: ArborTheme.background.withOpacity(0.7),
              child: const Center(
                child: CircularProgressIndicator(
                  color: ArborTheme.function,
                ),
              ),
            ),
          
          // Error overlay
          if (state.error != null)
             Positioned(
               top: 80,
               left: 20,
               right: 20,
               child: Material(
                 color: Colors.red.withOpacity(0.9),
                 borderRadius: BorderRadius.circular(8),
                 child: Padding(
                   padding: const EdgeInsets.all(12),
                   child: Row(
                     children: [
                       const Icon(Icons.error_outline, color: Colors.white),
                       const SizedBox(width: 12),
                       Expanded(child: Text(state.error!, style: const TextStyle(color: Colors.white))),
                       IconButton(
                         icon: const Icon(Icons.close, color: Colors.white),
                         onPressed: () => ref.read(graphProvider.notifier).connect(), // Retry
                       )
                     ],
                   ),
                 ),
               ),
             ),
        ],
      ),
    );
  }

  Widget _buildBackground() {
    return Container(
      decoration: const BoxDecoration(
        gradient: RadialGradient(
          center: Alignment.center,
          radius: 1.5,
          colors: [
            ArborTheme.surface,
            ArborTheme.background,
          ],
        ),
      ),
    );
  }

  Widget _buildTopBar(GraphState state) {
    return Positioned(
      top: 0,
      left: 0,
      right: 0,
      child: Container(
        padding: const EdgeInsets.all(16),
        decoration: BoxDecoration(
          gradient: LinearGradient(
            begin: Alignment.topCenter,
            end: Alignment.bottomCenter,
            colors: [
              ArborTheme.background,
              ArborTheme.background.withOpacity(0),
            ],
          ),
        ),
        child: Row(
          children: [
            // Logo
            Row(
              children: [
                Container(
                  width: 32,
                  height: 32,
                  decoration: BoxDecoration(
                    color: ArborTheme.function.withOpacity(0.2),
                    borderRadius: BorderRadius.circular(8),
                  ),
                  child: const Icon(
                    Icons.account_tree,
                    color: ArborTheme.function,
                    size: 20,
                  ),
                ),
                const SizedBox(width: 12),
                Text(
                  'Arbor',
                  style: Theme.of(context).textTheme.titleLarge,
                ),
              ],
            ),
            const SizedBox(width: 32),

            // Search
            Expanded(
              child: SizedBox(
                height: 40,
                child: TextField(
                  controller: _searchController,
                  onSubmitted: _handleSearch,
                  decoration: InputDecoration(
                    hintText: 'Search for functions, classes...',
                    prefixIcon: const Icon(
                      Icons.search,
                      color: ArborTheme.textMuted,
                      size: 20,
                    ),
                    suffixIcon: IconButton(
                      icon: const Icon(
                        Icons.arrow_forward,
                        color: ArborTheme.function,
                        size: 20,
                      ),
                      onPressed: () => _handleSearch(_searchController.text),
                    ),
                    contentPadding: const EdgeInsets.symmetric(horizontal: 16),
                  ),
                ),
              ),
            ),
            const SizedBox(width: 32),

            // Connection indicator
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
              decoration: BoxDecoration(
                color: state.isConnected
                    ? ArborTheme.method.withOpacity(0.2)
                    : ArborTheme.importType.withOpacity(0.2),
                borderRadius: BorderRadius.circular(16),
              ),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Container(
                    width: 8,
                    height: 8,
                    decoration: BoxDecoration(
                      color: state.isConnected
                          ? ArborTheme.method
                          : ArborTheme.importType,
                      shape: BoxShape.circle,
                    ),
                  ),
                  const SizedBox(width: 8),
                  Text(
                    state.isConnected ? 'Connected' : 'Disconnected',
                    style: TextStyle(
                      color: state.isConnected
                          ? ArborTheme.method
                          : ArborTheme.importType,
                      fontSize: 12,
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildInspector(GraphState state) {
    if (state.selectedNodeId == null) return const SizedBox.shrink();
    
    // Find node safely
    final node = state.nodes.firstWhere(
      (n) => n.id == state.selectedNodeId,
      orElse: () => state.nodes.isEmpty ? GraphNode(id: '', name: '', kind: '', file: '', lineStart: 0, lineEnd: 0) : state.nodes.first,
    );
    
    if (node.id.isEmpty) return const SizedBox.shrink();

    return Positioned(
      top: 80,
      right: 16,
      width: 320,
      child: Container(
        padding: const EdgeInsets.all(16),
        decoration: BoxDecoration(
          color: ArborTheme.surface,
          borderRadius: BorderRadius.circular(12),
          border: Border.all(color: ArborTheme.border),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          mainAxisSize: MainAxisSize.min,
          children: [
            // Header
            Row(
              children: [
                Container(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 8,
                    vertical: 4,
                  ),
                  decoration: BoxDecoration(
                    color: ArborTheme.colorForKind(node.kind).withOpacity(0.2),
                    borderRadius: BorderRadius.circular(4),
                  ),
                  child: Text(
                    node.kind.toUpperCase(),
                    style: TextStyle(
                      color: ArborTheme.colorForKind(node.kind),
                      fontSize: 10,
                      fontWeight: FontWeight.bold,
                    ),
                  ),
                ),
                const Spacer(),
                IconButton(
                  icon: const Icon(Icons.close, size: 18),
                  color: ArborTheme.textMuted,
                  onPressed: () {
                    ref.read(graphProvider.notifier).selectNode(null);
                  },
                ),
              ],
            ),
            const SizedBox(height: 12),

            // Name
            Text(
              node.name,
              style: Theme.of(context).textTheme.titleLarge,
            ),
            if (node.qualifiedName != null && node.qualifiedName != node.name)
              Text(
                node.qualifiedName!,
                style: Theme.of(context).textTheme.bodyMedium,
              ),
            const SizedBox(height: 16),

            // File location
            Row(
              children: [
                const Icon(
                  Icons.insert_drive_file_outlined,
                  size: 14,
                  color: ArborTheme.textMuted,
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    '${node.file}:${node.lineStart}',
                    style: Theme.of(context).textTheme.bodyMedium,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 8),

            // Signature
            if (node.signature != null) ...[
              Container(
                width: double.infinity,
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: ArborTheme.background,
                  borderRadius: BorderRadius.circular(8),
                ),
                child: Text(
                  node.signature!,
                  style: Theme.of(context).textTheme.bodyLarge,
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }



  Widget _buildSettingsPanel(GraphState state) {
    return Positioned(
      bottom: 48,
      right: 16,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
        decoration: BoxDecoration(
          color: ArborTheme.surface.withOpacity(0.9),
          borderRadius: BorderRadius.circular(12),
          border: Border.all(color: ArborTheme.border.withOpacity(0.5)),
          boxShadow: [
            BoxShadow(
              color: Colors.black.withOpacity(0.2),
              blurRadius: 8,
              offset: const Offset(0, 4),
            ),
          ],
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.end,
          mainAxisSize: MainAxisSize.min,
          children: [
            // Follow Toggle
            Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  'Follow AI',
                  style: Theme.of(context).textTheme.bodySmall?.copyWith(
                    color: state.isFollowMode ? ArborTheme.function : ArborTheme.textMuted,
                    fontWeight: state.isFollowMode ? FontWeight.bold : FontWeight.normal,
                  ),
                ),
                const SizedBox(width: 8),
                Switch(
                  value: state.isFollowMode,
                  activeColor: ArborTheme.function,
                  onChanged: (_) => ref.read(graphProvider.notifier).toggleFollowMode(),
                ),
              ],
            ),
            
            // Low GPU Toggle
            Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  'Low GPU',
                  style: Theme.of(context).textTheme.bodySmall?.copyWith(
                    color: state.isLowGpuMode ? ArborTheme.keyword : ArborTheme.textMuted,
                    fontWeight: state.isLowGpuMode ? FontWeight.bold : FontWeight.normal,
                  ),
                ),
                const SizedBox(width: 8),
                Switch(
                  value: state.isLowGpuMode,
                  activeColor: ArborTheme.keyword,
                  onChanged: (_) => ref.read(graphProvider.notifier).toggleLowGpuMode(),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }

  Widget _buildStatusBar(GraphState state) {
    return Positioned(
      bottom: 0,
      left: 0,
      right: 0,
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
        decoration: BoxDecoration(
          gradient: LinearGradient(
            begin: Alignment.bottomCenter,
            end: Alignment.topCenter,
            colors: [
              ArborTheme.background,
              ArborTheme.background.withOpacity(0),
            ],
          ),
        ),
        child: Row(
          children: [
            Text(
              '${state.nodes.length} nodes',
              style: Theme.of(context).textTheme.labelSmall,
            ),
            const SizedBox(width: 16),
            Text(
              '${state.edges.length} edges',
              style: Theme.of(context).textTheme.labelSmall,
            ),
            const Spacer(),
            Text(
              // Scale is internal to GraphWidget, not easily accessible here unless raised state.
              // For simplicity, just showing connection status or omitted.
              state.isConnected ? 'SYNCED' : 'OFFLINE',
               style: Theme.of(context).textTheme.labelSmall?.copyWith(
                 color: state.isConnected ? Colors.green : Colors.red,
                 fontWeight: FontWeight.bold,
                 letterSpacing: 2
               ),
            ),
          ],
        ),
      ),
    );
  }
}
