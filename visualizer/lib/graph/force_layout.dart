import 'dart:math';
import '../core/providers.dart';
import 'quad_tree.dart';

/// Force-directed layout algorithm with file-based clustering.
///
/// "The Multi-Layer Logic Forest"
/// - Files act as gravity wells (clusters).
/// - Symbols orbit their parent file.
/// - Edges represent relations.
class ForceLayout {
  // Physics Parameters
  static const double repulsion = 8000;
  static const double attraction = 0.08;
  static const double clusterGravity = 0.5; // Pull towards file center
  static const double damping = 0.92;
  static const double minDistance = 60;
  static const double maxForce = 50;

  /// Runs one iteration of the force simulation.
  static bool update(List<GraphNode> nodes, List<GraphEdge> edges, double dt) {
    if (nodes.isEmpty) return false;

    // 1. Build rapid lookups
    final nodeMap = {for (var n in nodes) n.id: n};
    final nodesByFile = <String, List<GraphNode>>{};
    
    // Group by file for clustering
    for (var node in nodes) {
      nodesByFile.putIfAbsent(node.file, () => []).add(node);
    }

    // 2. File Clustering (Gravity Wells)
    // Calculate centroid of each file and pull nodes towards it
    nodesByFile.forEach((file, fileNodes) {
      if (fileNodes.length <= 1) return;

      var cx = 0.0;
      var cy = 0.0;
      for (var n in fileNodes) {
        cx += n.x;
        cy += n.y;
      }
      cx /= fileNodes.length;
      cy /= fileNodes.length;

      for (var n in fileNodes) {
        final dx = cx - n.x;
        final dy = cy - n.y;
        final dist = sqrt(dx * dx + dy * dy);
        
        if (dist > minDistance / 2) {
          final force = dist * clusterGravity;
          n.vx += (dx / dist) * force * dt;
          n.vy += (dy / dist) * force * dt;
        }
      }
    });

    // 3. Repulsion using Barnes-Hut approximation (O(n log n))
    // Build QuadTree for spatial partitioning
    final quadTree = QuadTree.build(nodes);
    
    for (final node in nodes) {
      final (fx, fy) = quadTree.calculateForce(node, repulsion);
      node.vx += fx * dt;
      node.vy += fy * dt;
    }

    // 4. Edge Attraction (Springs)
    for (final edge in edges) {
      final a = nodeMap[edge.source]; // Note: used 'source'/'target' from protocol
      final b = nodeMap[edge.target];
      if (a == null || b == null) continue;
      if (a == b) continue;

      final dx = b.x - a.x;
      final dy = b.y - a.y;
      final dist = sqrt(dx * dx + dy * dy);

      if (dist > minDistance) {
        final force = (dist - minDistance) * attraction;
        // Cap force
        final f = min(force, maxForce);
        
        final fx = (dx / dist) * f;
        final fy = (dy / dist) * f;

        a.vx += fx * dt;
        a.vy += fy * dt;
        b.vx -= fx * dt;
        b.vy -= fy * dt;
      }
    }

    // 5. Integration (Apply Velocity)
    var totalEnergy = 0.0;
    for (final node in nodes) {
      // Damping
      node.vx *= damping;
      node.vy *= damping;

      // Update position
      node.x += node.vx * dt;
      node.y += node.vy * dt;

      totalEnergy += node.vx * node.vx + node.vy * node.vy;
    }

    // Return true if simulation is active (energy > threshold)
    return totalEnergy > 0.1;
  }

  /// Centers the graph in the given bounds.
  static void centerNodes(List<GraphNode> nodes, double width, double height) {
    if (nodes.isEmpty) return;
    
    var cx = 0.0;
    var cy = 0.0;
    for (var node in nodes) {
      cx += node.x;
      cy += node.y;
    }
    cx /= nodes.length;
    cy /= nodes.length;
    
    final dx = width / 2 - cx;
    final dy = height / 2 - cy;
    
    for (var node in nodes) {
      node.x += dx;
      node.y += dy;
    }
  }
}
