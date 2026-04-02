import 'dart:math';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import '../services/websocket_service.dart';
import 'protocol.dart';

// Export protocol classes for consumers
export 'protocol.dart';

/// Provider for the WebSocket service.
final webSocketServiceProvider = Provider<WebSocketService>((ref) {
  final service = WebSocketService();
  ref.onDispose(() => service.dispose());
  return service;
});

/// State of the graph visualization.
class GraphState {
  final List<GraphNode> nodes;
  final List<GraphEdge> edges;
  final bool isConnected;
  final bool isLoading;
  final String? error;
  final String? selectedNodeId;
  
  /// Node ID currently spotlighted by AI (triggers camera animation)
  final String? spotlightNodeId;

  // Stats from server
  final int fileCount;

  // Settings
  final bool isLowGpuMode;
  final bool isFollowMode;

  const GraphState({
    this.nodes = const [],
    this.edges = const [],
    this.isConnected = false,
    this.isLoading = false,
    this.error,
    this.selectedNodeId,
    this.spotlightNodeId,
    this.fileCount = 0,
    this.isLowGpuMode = false,
    this.isFollowMode = true,
  });

  GraphState copyWith({
    List<GraphNode>? nodes,
    List<GraphEdge>? edges,
    bool? isConnected,
    bool? isLoading,
    String? error,
    String? selectedNodeId,
    String? spotlightNodeId,
    int? fileCount,
    bool? isLowGpuMode,
    bool? isFollowMode,
  }) {
    return GraphState(
      nodes: nodes ?? this.nodes,
      edges: edges ?? this.edges,
      isConnected: isConnected ?? this.isConnected,
      isLoading: isLoading ?? this.isLoading,
      error: error,
      selectedNodeId: selectedNodeId ?? this.selectedNodeId,
      spotlightNodeId: spotlightNodeId,
      fileCount: fileCount ?? this.fileCount,
      isLowGpuMode: isLowGpuMode ?? this.isLowGpuMode,
      isFollowMode: isFollowMode ?? this.isFollowMode,
    );
  }
}

/// Provider for graph state management.
class GraphNotifier extends StateNotifier<GraphState> {
  final WebSocketService _wsService;

  GraphNotifier(this._wsService) : super(const GraphState()) {
    // Listen to incoming messages
    _wsService.messageStream.listen(_handleMessage);
    
    // Check connection status
    // ideally wsService exposes a stream of connection status too, but for MVP we infer from events
  }

  /// Connects to the Arbor server.
  Future<void> connect() async {
    state = state.copyWith(isLoading: true, error: null);
    await _wsService.connect('ws://127.0.0.1:8081'); // Use SyncServer port
    state = state.copyWith(isConnected: _wsService.isConnected, isLoading: false);
  }

  void selectNode(String? id) {
    state = state.copyWith(selectedNodeId: id);
  }

  void _handleMessage(BroadcastMessage message) {
    if (message is GraphUpdate) {
      if (message.nodes != null && message.edges != null) {
        // Full update
         final nodes = message.nodes!;
         final edges = message.edges!;
         
         // Merge with existing nodes to preserve positions
         final existingNodes = {for (var n in state.nodes) n.id: n};
         
         for (var node in nodes) {
           if (existingNodes.containsKey(node.id)) {
             final existing = existingNodes[node.id]!;
             node.x = existing.x;
             node.y = existing.y;
             node.vx = existing.vx;
             node.vy = existing.vy;
             node.isHovered = existing.isHovered;
           } else {
             // New node - spawn near center
             node.x = 400 + (Random().nextDouble() - 0.5) * 100;
             node.y = 300 + (Random().nextDouble() - 0.5) * 100;
           }
         }

         state = state.copyWith(
           nodes: nodes,
           edges: edges,
           fileCount: message.fileCount,
         );
      } else {
        // Delta update (or just stats if nodes are null)
        // For MVP, if nodes are null, we don't update graph structure, just stats
        state = state.copyWith(fileCount: message.fileCount);
      }
    } else if (message is FocusNode) {
      // AI is looking at this node - set spotlight for camera animation
      state = state.copyWith(
        selectedNodeId: message.nodeId,
        spotlightNodeId: message.nodeId,
      );
    }
  }

  void search(String query) {
     if (query.isEmpty) return;
     // Simple client-side search for now
     try {
       final match = state.nodes.firstWhere(
         (n) => n.name.toLowerCase().contains(query.toLowerCase()),
       );
       state = state.copyWith(selectedNodeId: match.id);
     } catch (e) {
       // No match found
     }
  }

  void toggleLowGpuMode() {
    state = state.copyWith(isLowGpuMode: !state.isLowGpuMode);
  }

  void toggleFollowMode() {
    state = state.copyWith(isFollowMode: !state.isFollowMode);
  }
}

/// Provider for graph state.
final graphProvider = StateNotifierProvider<GraphNotifier, GraphState>((ref) {
  final wsService = ref.watch(webSocketServiceProvider);
  return GraphNotifier(wsService);
});
