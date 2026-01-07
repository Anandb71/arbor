import 'dart:math';
import '../core/providers.dart';

/// A QuadTree for spatial partitioning of graph nodes.
/// Used by Barnes-Hut algorithm for O(n log n) force calculation.
class QuadTree {
  final double x, y, size;
  final List<GraphNode> nodes = [];
  QuadTree? nw, ne, sw, se;
  
  // Barnes-Hut threshold (theta). Higher = faster but less accurate.
  // Typical values: 0.5 (balanced), 0.8 (fast), 0.3 (accurate)
  static const double theta = 0.7;
  static const int maxNodesPerCell = 4;
  
  // Mass and center of mass for this cell
  double totalMass = 0;
  double centerX = 0;
  double centerY = 0;

  QuadTree(this.x, this.y, this.size);

  /// Checks if a point is within this quadrant.
  bool contains(double px, double py) {
    return px >= x && px < x + size && py >= y && py < y + size;
  }

  /// Inserts a node into the QuadTree.
  void insert(GraphNode node) {
    if (!contains(node.x, node.y)) return;
    
    // Update center of mass
    final newMass = totalMass + 1;
    centerX = (centerX * totalMass + node.x) / newMass;
    centerY = (centerY * totalMass + node.y) / newMass;
    totalMass = newMass;
    
    // If we have children, insert into appropriate child
    if (nw != null) {
      _insertIntoChild(node);
      return;
    }
    
    // Add to this cell
    nodes.add(node);
    
    // Subdivide if too many nodes
    if (nodes.length > maxNodesPerCell && size > 10) {
      _subdivide();
    }
  }

  void _subdivide() {
    final half = size / 2;
    nw = QuadTree(x, y, half);
    ne = QuadTree(x + half, y, half);
    sw = QuadTree(x, y + half, half);
    se = QuadTree(x + half, y + half, half);
    
    // Redistribute existing nodes
    for (final node in nodes) {
      _insertIntoChild(node);
    }
    nodes.clear();
  }

  void _insertIntoChild(GraphNode node) {
    if (nw!.contains(node.x, node.y)) {
      nw!.insert(node);
    } else if (ne!.contains(node.x, node.y)) {
      ne!.insert(node);
    } else if (sw!.contains(node.x, node.y)) {
      sw!.insert(node);
    } else if (se!.contains(node.x, node.y)) {
      se!.insert(node);
    }
  }

  /// Calculates repulsion force on a node using Barnes-Hut approximation.
  /// Returns (fx, fy) force vector.
  (double, double) calculateForce(GraphNode node, double repulsion) {
    if (totalMass == 0) return (0, 0);
    
    final dx = centerX - node.x;
    final dy = centerY - node.y;
    final distSq = dx * dx + dy * dy;
    
    // Prevent self-interaction and extreme forces
    if (distSq < 1) {
      // Check if this cell only contains the same node
      if (nodes.length == 1 && nodes.first.id == node.id) {
        return (0, 0);
      }
      // Add small random jitter to prevent clustering
      final r = Random();
      return ((r.nextDouble() - 0.5) * 0.1, (r.nextDouble() - 0.5) * 0.1);
    }
    
    final dist = sqrt(distSq);
    
    // Barnes-Hut approximation: if the cell is far enough away,
    // treat all nodes in it as a single mass at center of mass
    if (size / dist < theta || nw == null) {
      // Apply force from this cell's center of mass
      final force = (repulsion * totalMass) / distSq;
      final fx = -(dx / dist) * force;
      final fy = -(dy / dist) * force;
      return (fx, fy);
    }
    
    // Otherwise, recursively calculate force from children
    var fx = 0.0, fy = 0.0;
    for (final child in [nw, ne, sw, se]) {
      if (child != null && child.totalMass > 0) {
        final (cfx, cfy) = child.calculateForce(node, repulsion);
        fx += cfx;
        fy += cfy;
      }
    }
    return (fx, fy);
  }

  /// Builds a QuadTree from a list of nodes.
  static QuadTree build(List<GraphNode> nodes) {
    if (nodes.isEmpty) {
      return QuadTree(0, 0, 1000);
    }
    
    // Find bounding box
    var minX = nodes.first.x, maxX = nodes.first.x;
    var minY = nodes.first.y, maxY = nodes.first.y;
    
    for (final node in nodes) {
      minX = min(minX, node.x);
      maxX = max(maxX, node.x);
      minY = min(minY, node.y);
      maxY = max(maxY, node.y);
    }
    
    // Create square bounding box with padding
    final width = maxX - minX + 100;
    final height = maxY - minY + 100;
    final size = max(width, height);
    
    final tree = QuadTree(minX - 50, minY - 50, size);
    for (final node in nodes) {
      tree.insert(node);
    }
    
    return tree;
  }
}
