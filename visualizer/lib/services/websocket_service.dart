import 'dart:async';
import 'dart:convert';
import 'dart:math';
import 'package:web_socket_channel/web_socket_channel.dart';
import 'package:flutter/foundation.dart';
import '../core/protocol.dart';

class WebSocketService {
  WebSocketChannel? _channel;
  final StreamController<BroadcastMessage> _controller = StreamController.broadcast();
  bool _isConnected = false;
  bool _isDisposed = false;

  Stream<BroadcastMessage> get messageStream => _controller.stream;
  bool get isConnected => _isConnected;

  Future<void> connect(String url) async {
    if (_isConnected) return;
    
    int retryCount = 0;
    while (!_isConnected && !_isDisposed) {
      try {
        debugPrint('Connecting to $url...');
        final uri = Uri.parse(url);
        _channel = WebSocketChannel.connect(uri);
        await _channel!.ready;
        _isConnected = true;
        retryCount = 0;
        debugPrint('Connected to Arbor Server');

        _channel!.stream.listen(
          (message) {
            _handleMessage(message);
          },
          onDone: () {
            debugPrint('WebSocket connection closed');
            _isConnected = false;
            _reconnect(url);
          },
          onError: (error) {
            debugPrint('WebSocket error: $error');
            _isConnected = false;
            _reconnect(url);
          },
        );
      } catch (e) {
        debugPrint('Connection failed: $e');
        _isConnected = false;
        await _backoff(retryCount++);
      }
    }
  }

  Future<void> _reconnect(String url) async {
    if (_isDisposed) return;
    _channel = null;
    await _backoff(0);
    connect(url);
  }

  Future<void> _backoff(int retryCount) async {
    if (_isDisposed) return;
    final delay = min(30, pow(2, retryCount).toInt());
    debugPrint('Retrying in $delay seconds...');
    await Future.delayed(Duration(seconds: delay));
  }

  // Buffer for batch loading
  final List<GraphNode> _pendingNodes = [];
  final List<GraphEdge> _pendingEdges = [];

  void _handleMessage(dynamic message) {
    try {
      if (message is String) {
        final json = jsonDecode(message);
        // Check if it matches BroadcastMessage structure
        if (json is Map<String, dynamic> && json.containsKey('type')) {
           final type = json['type'];

           // 1. Handshake
           if (type == 'Hello') {
             final hello = Hello(json['payload']);
             debugPrint('👋 Handshake from server: v${hello.version}, expecting ${hello.nodeCount} nodes');
             // Send Ready
             _send({'type': 'ready_for_graph'});
             return;
           }

           // 2. Stream Control
           if (type == 'GraphBegin') {
             _pendingNodes.clear();
             _pendingEdges.clear();
             debugPrint('📥 Starting graph stream...');
             return;
           }

           if (type == 'GraphEnd') {
             debugPrint('🏁 Graph stream complete. Emitting update with ${_pendingNodes.length} nodes.');
             final update = GraphUpdate({
               'is_delta': false,
               'node_count': _pendingNodes.length,
               'edge_count': _pendingEdges.length,
               'file_count': 0,
               'changed_files': [],
               'timestamp': DateTime.now().millisecondsSinceEpoch,
               'nodes': _pendingNodes.map((n) => {
                 'id': n.id,
                 'name': n.name,
                 'kind': n.kind,
                 'file': n.file,
                 'start_line': n.lineStart,
                 'end_line': n.lineEnd,
                 'centrality': n.centrality,
               }).toList(),
               'edges': _pendingEdges.map((e) => {
                 'source': e.source,
                 'target': e.target,
                 'kind': e.kind,
               }).toList(),
             });
             _controller.add(update);
             return;
           }

           // 3. Batches
           if (type == 'NodeBatch') {
             final batch = NodeBatch(json['payload']);
             _pendingNodes.addAll(batch.nodes);
             return;
           }

           if (type == 'EdgeBatch') {
             final batch = EdgeBatch(json['payload']);
             _pendingEdges.addAll(batch.edges);
             return;
           }

           // 4. Legacy / Other
           final broadcast = BroadcastMessage.fromJson(json);
           _controller.add(broadcast);
        }
      }
    } catch (e, stack) {
      debugPrint('Error parsing message: $e\n$stack');
    }
  }

  void _send(Map<String, dynamic> data) {
    if (_channel != null && _channel!.closeCode == null) {
      _channel!.sink.add(jsonEncode(data));
    }
  }

  void dispose() {
    _isDisposed = true;
    _channel?.sink.close();
    _controller.close();
  }
}
