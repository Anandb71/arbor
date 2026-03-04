/// Represents a BroadcastMessage from the server.
sealed class BroadcastMessage {
  final String type;

  BroadcastMessage(this.type);

  factory BroadcastMessage.fromJson(Map<String, dynamic> json) {
    final type = json['type'] as String;
    final payload = json['payload'] as Map<String, dynamic>;

    switch (type) {
      case 'Hello':
        return Hello(payload);
      case 'GraphBegin':
        return GraphBegin(payload);
      case 'NodeBatch':
        return NodeBatch(payload);
      case 'EdgeBatch':
        return EdgeBatch(payload);
      case 'GraphEnd':
        return GraphEnd();
      case 'GraphUpdate':
        return GraphUpdate(payload);
      case 'FocusNode':
        return FocusNode(payload);
      case 'IndexerStatus':
        return IndexerStatus(payload);
      default:
        throw Exception('Unknown message type: $type');
    }
  }
}

class Hello extends BroadcastMessage {
  final String version;
  final int nodeCount;
  final int edgeCount;

  Hello(Map<String, dynamic> json)
      : version = json['version'] as String,
        nodeCount = json['node_count'] as int,
        edgeCount = json['edge_count'] as int,
        super('Hello');
}

class GraphBegin extends BroadcastMessage {
  final int totalNodes;
  final int totalEdges;

  GraphBegin(Map<String, dynamic> json)
      : totalNodes = json['total_nodes'] as int,
        totalEdges = json['total_edges'] as int,
        super('GraphBegin');
}

class NodeBatch extends BroadcastMessage {
  final List<GraphNode> nodes;

  NodeBatch(Map<String, dynamic> json)
      : nodes = (json['nodes'] as List)
            .map((e) => GraphNode.fromJson(e))
            .toList(),
        super('NodeBatch');
}

class EdgeBatch extends BroadcastMessage {
  final List<GraphEdge> edges;

  EdgeBatch(Map<String, dynamic> json)
      : edges = (json['edges'] as List)
            .map((e) => GraphEdge.fromJson(e))
            .toList(),
        super('EdgeBatch');
}

class GraphEnd extends BroadcastMessage {
  GraphEnd() : super('GraphEnd');
}

class GraphUpdate extends BroadcastMessage {
  final bool isDelta;
  final int nodeCount;
  final int edgeCount;
  final int fileCount;
  final List<String> changedFiles;
  final int timestamp;
  final List<GraphNode>? nodes;
  final List<GraphEdge>? edges;

  GraphUpdate(Map<String, dynamic> json)
      : isDelta = json['is_delta'] as bool,
        nodeCount = json['node_count'] as int,
        edgeCount = json['edge_count'] as int,
        fileCount = json['file_count'] as int,
        changedFiles = (json['changed_files'] as List).cast<String>(),
        timestamp = json['timestamp'] as int,
        nodes = json['nodes'] != null
            ? (json['nodes'] as List).map((e) => GraphNode.fromJson(e)).toList()
            : null,
        edges = json['edges'] != null
            ? (json['edges'] as List).map((e) => GraphEdge.fromJson(e)).toList()
            : null,
        super('GraphUpdate');
}

class FocusNode extends BroadcastMessage {
  final String nodeId;
  final String file;
  final int line;

  FocusNode(Map<String, dynamic> json)
      : nodeId = json['node_id'] as String,
        file = json['file'] as String,
        line = json['line'] as int,
        super('FocusNode');
}

class IndexerStatus extends BroadcastMessage {
  final String phase;
  final int filesProcessed;
  final int filesTotal;
  final String? currentFile;

  IndexerStatus(Map<String, dynamic> json)
      : phase = json['phase'] as String,
        filesProcessed = json['files_processed'] as int,
        filesTotal = json['files_total'] as int,
        currentFile = json['current_file'] as String?,
        super('IndexerStatus');
}

/// Represents a node in the code graph.
class GraphNode {
  final String id;
  final String name;
  final String kind; // function, class, etc.
  final String file;
  final int lineStart;
  final int lineEnd;
  final String? qualifiedName;
  final String? signature;
  final double centrality;

  // UI / Simulation State
  double x = 0;
  double y = 0;
  double vx = 0;
  double vy = 0;
  bool isHovered = false;

  GraphNode({
    required this.id,
    required this.name,
    required this.kind,
    required this.file,
    required this.lineStart,
    required this.lineEnd,
    this.qualifiedName,
    this.signature,
    this.centrality = 0.0,
  });

  factory GraphNode.fromJson(Map<String, dynamic> json) {
    return GraphNode(
      id: json['id'] as String,
      name: json['name'] as String,
      kind: json['kind'] as String,
      file: json['file'] as String,
      lineStart: json['start_line'] ?? 0,
      lineEnd: json['end_line'] ?? 0,
      qualifiedName: json['qualified_name'] ?? json['name'],
      signature: json['signature'],
      centrality: (json['centrality'] ?? 0).toDouble(),
    );
  }
}

/// Represents an edge between nodes.
class GraphEdge {
  final String source;
  final String target;
  final String kind; // calls, imports, defines

  GraphEdge({
    required this.source,
    required this.target,
    required this.kind,
  });

  factory GraphEdge.fromJson(Map<String, dynamic> json) {
    return GraphEdge(
      source: json['source'] as String,
      target: json['target'] as String,
      kind: json['kind'] ?? 'calls',
    );
  }
}
