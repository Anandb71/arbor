import 'package:flutter_test/flutter_test.dart';

import 'package:arbor_visualizer/core/providers.dart';
import 'package:arbor_visualizer/core/protocol.dart' as protocol;

void main() {
  group('GraphState', () {
    test('initial state has empty nodes and edges', () {
      const state = GraphState();
      expect(state.nodes, isEmpty);
      expect(state.edges, isEmpty);
      expect(state.isConnected, isFalse);
      expect(state.isLoading, isFalse);
      expect(state.error, isNull);
      expect(state.selectedNodeId, isNull);
      expect(state.spotlightNodeId, isNull);
    });

    test('copyWith preserves unchanged values', () {
      const state = GraphState(
        isConnected: true,
        fileCount: 10,
      );

      final newState = state.copyWith(isLoading: true);

      expect(newState.isConnected, isTrue);
      expect(newState.fileCount, 10);
      expect(newState.isLoading, isTrue);
    });

    test('copyWith can update nodes and edges', () {
      const state = GraphState();
      final nodes = [
        protocol.GraphNode(
          id: 'test-1',
          name: 'testFunc',
          kind: 'function',
          file: 'test.rs',
          lineStart: 1,
          lineEnd: 10,
        ),
      ];
      final edges = [
        protocol.GraphEdge(source: 'test-1', target: 'test-2', kind: 'calls'),
      ];

      final newState = state.copyWith(nodes: nodes, edges: edges);

      expect(newState.nodes.length, 1);
      expect(newState.nodes.first.name, 'testFunc');
      expect(newState.edges.length, 1);
    });

    test('copyWith clears error when set to null', () {
      final state = const GraphState().copyWith(error: 'Some error');
      expect(state.error, 'Some error');

      final clearedState = state.copyWith(error: null);
      expect(clearedState.error, isNull);
    });

    test('isFollowMode defaults to true', () {
      const state = GraphState();
      expect(state.isFollowMode, isTrue);
    });

    test('isLowGpuMode defaults to false', () {
      const state = GraphState();
      expect(state.isLowGpuMode, isFalse);
    });
  });

  group('GraphNode', () {
    test('creates node with required fields', () {
      final node = protocol.GraphNode(
        id: 'node-1',
        name: 'myFunction',
        kind: 'function',
        file: 'src/lib.rs',
        lineStart: 10,
        lineEnd: 20,
      );

      expect(node.id, 'node-1');
      expect(node.name, 'myFunction');
      expect(node.kind, 'function');
      expect(node.file, 'src/lib.rs');
      expect(node.lineStart, 10);
      expect(node.lineEnd, 20);
    });

    test('node has default position at origin', () {
      final node = protocol.GraphNode(
        id: 'node-1',
        name: 'test',
        kind: 'function',
        file: 'test.rs',
        lineStart: 1,
        lineEnd: 5,
      );

      expect(node.x, 0.0);
      expect(node.y, 0.0);
      expect(node.vx, 0.0);
      expect(node.vy, 0.0);
    });

    test('node has default centrality of 0', () {
      final node = protocol.GraphNode(
        id: 'node-1',
        name: 'test',
        kind: 'function',
        file: 'test.rs',
        lineStart: 1,
        lineEnd: 5,
      );

      expect(node.centrality, 0.0);
    });

    test('fromJson parses node correctly', () {
      final json = {
        'id': 'node-123',
        'name': 'parseData',
        'kind': 'function',
        'file': 'parser.rs',
        'start_line': 50,
        'end_line': 100,
        'qualified_name': 'parser::parseData',
        'signature': 'fn parseData(input: &str) -> Result<Data>',
        'centrality': 0.75,
      };

      final node = GraphNode.fromJson(json);

      expect(node.id, 'node-123');
      expect(node.name, 'parseData');
      expect(node.kind, 'function');
      expect(node.file, 'parser.rs');
      expect(node.lineStart, 50);
      expect(node.lineEnd, 100);
      expect(node.qualifiedName, 'parser::parseData');
      expect(node.signature, 'fn parseData(input: &str) -> Result<Data>');
      expect(node.centrality, 0.75);
    });

    test('fromJson handles missing optional fields', () {
      final json = {
        'id': 'node-456',
        'name': 'simpleFunc',
        'kind': 'function',
        'file': 'simple.rs',
      };

      final node = GraphNode.fromJson(json);

      expect(node.id, 'node-456');
      expect(node.name, 'simpleFunc');
      expect(node.lineStart, 0);
      expect(node.lineEnd, 0);
      expect(node.centrality, 0.0);
    });
  });

  group('GraphEdge', () {
    test('creates edge with required fields', () {
      final edge = protocol.GraphEdge(
        source: 'node-1',
        target: 'node-2',
        kind: 'calls',
      );

      expect(edge.source, 'node-1');
      expect(edge.target, 'node-2');
      expect(edge.kind, 'calls');
    });

    test('fromJson parses edge correctly', () {
      final json = {
        'source': 'func-a',
        'target': 'func-b',
        'kind': 'imports',
      };

      final edge = GraphEdge.fromJson(json);

      expect(edge.source, 'func-a');
      expect(edge.target, 'func-b');
      expect(edge.kind, 'imports');
    });

    test('fromJson defaults kind to calls when missing', () {
      final json = {
        'source': 'func-a',
        'target': 'func-b',
      };

      final edge = GraphEdge.fromJson(json);

      expect(edge.kind, 'calls');
    });
  });

  group('BroadcastMessage', () {
    test('parses Hello message', () {
      final json = {
        'type': 'Hello',
        'payload': {
          'version': '2.0',
          'node_count': 100,
          'edge_count': 250,
        },
      };

      final message = protocol.BroadcastMessage.fromJson(json);

      expect(message, isA<protocol.Hello>());
      final hello = message as protocol.Hello;
      expect(hello.version, '2.0');
      expect(hello.nodeCount, 100);
      expect(hello.edgeCount, 250);
    });

    test('parses FocusNode message', () {
      final json = {
        'type': 'FocusNode',
        'payload': {
          'node_id': 'focus-123',
          'file': 'main.rs',
          'line': 42,
        },
      };

      final message = protocol.BroadcastMessage.fromJson(json);

      expect(message, isA<protocol.FocusNode>());
      final focus = message as protocol.FocusNode;
      expect(focus.nodeId, 'focus-123');
      expect(focus.file, 'main.rs');
      expect(focus.line, 42);
    });

    test('parses IndexerStatus message', () {
      final json = {
        'type': 'IndexerStatus',
        'payload': {
          'phase': 'indexing',
          'files_processed': 50,
          'files_total': 100,
          'current_file': 'lib.rs',
        },
      };

      final message = protocol.BroadcastMessage.fromJson(json);

      expect(message, isA<protocol.IndexerStatus>());
      final status = message as protocol.IndexerStatus;
      expect(status.phase, 'indexing');
      expect(status.filesProcessed, 50);
      expect(status.filesTotal, 100);
      expect(status.currentFile, 'lib.rs');
    });
  });
}
