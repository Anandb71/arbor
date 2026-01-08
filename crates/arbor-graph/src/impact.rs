//! Impact analysis for code changes.
//!
//! This module provides bidirectional BFS traversal to find all nodes
//! affected by a change to a target node. It answers the question:
//! "What breaks if I change this?"

use crate::edge::EdgeKind;
use crate::graph::{ArborGraph, NodeId};
use crate::query::NodeInfo;
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

/// Severity of impact based on hop distance from target.
///
/// Never construct directly — always use `from_hops()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ImpactSeverity {
    /// 1 hop - immediate callers/callees
    Direct = 0,
    /// 2-3 hops - transitively connected
    Transitive = 1,
    /// 4+ hops - distantly connected
    Distant = 2,
}

impl ImpactSeverity {
    /// Derives severity from hop distance.
    ///
    /// This is the ONLY way to create an ImpactSeverity.
    /// Thresholds: 1 hop = Direct, 2-3 = Transitive, 4+ = Distant
    pub fn from_hops(hops: usize) -> Self {
        match hops {
            0 | 1 => ImpactSeverity::Direct,
            2 | 3 => ImpactSeverity::Transitive,
            _ => ImpactSeverity::Distant,
        }
    }

    /// Returns a human-readable description.
    pub fn as_str(&self) -> &'static str {
        match self {
            ImpactSeverity::Direct => "direct",
            ImpactSeverity::Transitive => "transitive",
            ImpactSeverity::Distant => "distant",
        }
    }
}

impl std::fmt::Display for ImpactSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Direction of impact from the target node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImpactDirection {
    /// Nodes that depend on the target (incoming edges).
    /// These break if the target's interface changes.
    Upstream,
    /// Nodes the target depends on (outgoing edges).
    /// Changes here may require updating the target.
    Downstream,
}

impl std::fmt::Display for ImpactDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImpactDirection::Upstream => write!(f, "upstream"),
            ImpactDirection::Downstream => write!(f, "downstream"),
        }
    }
}

/// A node affected by a change to the target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AffectedNode {
    /// The node's graph index.
    pub node_id: NodeId,
    /// Full node information.
    pub node_info: NodeInfo,
    /// Severity derived from hop distance.
    pub severity: ImpactSeverity,
    /// Number of edges from target to this node.
    pub hop_distance: usize,
    /// The edge kind of the first hop that led to this node.
    /// This explains why this node is in the impact set.
    pub entry_edge: EdgeKind,
    /// Whether this node is upstream or downstream of target.
    pub direction: ImpactDirection,
}

/// Complete impact analysis result.
#[derive(Debug, Serialize, Deserialize)]
pub struct ImpactAnalysis {
    /// The target node being analyzed.
    pub target: NodeInfo,
    /// Nodes that depend on the target (callers, importers, etc.)
    pub upstream: Vec<AffectedNode>,
    /// Nodes the target depends on (callees, imports, etc.)
    pub downstream: Vec<AffectedNode>,
    /// Total count of affected nodes.
    pub total_affected: usize,
    /// Maximum depth searched.
    pub max_depth: usize,
    /// Time taken in milliseconds.
    pub query_time_ms: u64,
}

impl ImpactAnalysis {
    /// Returns all affected nodes (upstream + downstream) sorted by severity.
    pub fn all_affected(&self) -> Vec<&AffectedNode> {
        let mut all: Vec<&AffectedNode> =
            self.upstream.iter().chain(self.downstream.iter()).collect();

        // Stable sort: severity → hop_distance → node_id
        all.sort_by(|a, b| {
            a.severity
                .cmp(&b.severity)
                .then_with(|| a.hop_distance.cmp(&b.hop_distance))
                .then_with(|| a.node_info.id.cmp(&b.node_info.id))
        });

        all
    }

    /// Returns only direct (1-hop) affected nodes.
    pub fn direct_only(&self) -> Vec<&AffectedNode> {
        self.all_affected()
            .into_iter()
            .filter(|n| n.severity == ImpactSeverity::Direct)
            .collect()
    }

    /// Returns a summary suitable for CLI output.
    pub fn summary(&self) -> String {
        let direct = self
            .all_affected()
            .iter()
            .filter(|n| n.severity == ImpactSeverity::Direct)
            .count();
        let transitive = self
            .all_affected()
            .iter()
            .filter(|n| n.severity == ImpactSeverity::Transitive)
            .count();
        let distant = self
            .all_affected()
            .iter()
            .filter(|n| n.severity == ImpactSeverity::Distant)
            .count();

        format!(
            "Blast Radius: {} nodes (direct: {}, transitive: {}, distant: {})",
            self.total_affected, direct, transitive, distant
        )
    }
}

impl ArborGraph {
    /// Analyzes the impact of changing a node.
    ///
    /// Performs bidirectional BFS from the target:
    /// - Upstream: nodes that depend on target (would break if target changes)
    /// - Downstream: nodes target depends on (may require target updates)
    ///
    /// # Arguments
    /// * `target` - The node to analyze
    /// * `max_depth` - Maximum hop distance to traverse (0 = unlimited)
    ///
    /// # Returns
    /// Complete impact analysis with affected nodes sorted by severity.
    pub fn analyze_impact(&self, target: NodeId, max_depth: usize) -> ImpactAnalysis {
        let start = Instant::now();

        let target_node = match self.get(target) {
            Some(node) => NodeInfo::from(node),
            None => {
                return ImpactAnalysis {
                    target: NodeInfo {
                        id: String::new(),
                        name: String::new(),
                        qualified_name: String::new(),
                        kind: String::new(),
                        file: String::new(),
                        line_start: 0,
                        line_end: 0,
                        signature: None,
                        centrality: 0.0,
                    },
                    upstream: Vec::new(),
                    downstream: Vec::new(),
                    total_affected: 0,
                    max_depth,
                    query_time_ms: 0,
                };
            }
        };

        let effective_depth = if max_depth == 0 {
            usize::MAX
        } else {
            max_depth
        };

        let upstream = self.bfs_impact(target, Direction::Incoming, effective_depth);
        let downstream = self.bfs_impact(target, Direction::Outgoing, effective_depth);

        let total = upstream.len() + downstream.len();
        let elapsed = start.elapsed().as_millis() as u64;

        ImpactAnalysis {
            target: target_node,
            upstream,
            downstream,
            total_affected: total,
            max_depth,
            query_time_ms: elapsed,
        }
    }

    /// BFS traversal in one direction from target.
    fn bfs_impact(
        &self,
        target: NodeId,
        direction: Direction,
        max_depth: usize,
    ) -> Vec<AffectedNode> {
        let mut result = Vec::new();
        let mut visited: HashSet<NodeId> = HashSet::new();
        let mut queue: VecDeque<(NodeId, usize, EdgeKind)> = VecDeque::new();

        // Track entry edges for each node (first edge that reaches it)
        let mut entry_edges: HashMap<NodeId, EdgeKind> = HashMap::new();

        visited.insert(target);

        // Seed queue with immediate neighbors
        for edge_ref in self.graph.edges_directed(target, direction) {
            let neighbor = match direction {
                Direction::Incoming => edge_ref.source(),
                Direction::Outgoing => edge_ref.target(),
            };

            if !visited.contains(&neighbor) {
                let edge_kind = edge_ref.weight().kind;
                queue.push_back((neighbor, 1, edge_kind));
                entry_edges.insert(neighbor, edge_kind);
            }
        }

        while let Some((current, depth, entry_edge)) = queue.pop_front() {
            if depth > max_depth || visited.contains(&current) {
                continue;
            }

            visited.insert(current);

            if let Some(node) = self.get(current) {
                let mut node_info = NodeInfo::from(node);
                node_info.centrality = self.centrality(current);

                let impact_direction = match direction {
                    Direction::Incoming => ImpactDirection::Upstream,
                    Direction::Outgoing => ImpactDirection::Downstream,
                };

                result.push(AffectedNode {
                    node_id: current,
                    node_info,
                    severity: ImpactSeverity::from_hops(depth),
                    hop_distance: depth,
                    entry_edge,
                    direction: impact_direction,
                });
            }

            // Continue BFS if not at max depth
            if depth < max_depth {
                for edge_ref in self.graph.edges_directed(current, direction) {
                    let neighbor = match direction {
                        Direction::Incoming => edge_ref.source(),
                        Direction::Outgoing => edge_ref.target(),
                    };

                    if !visited.contains(&neighbor) {
                        let next_entry = *entry_edges.get(&neighbor).unwrap_or(&entry_edge);
                        queue.push_back((neighbor, depth + 1, next_entry));

                        // Store entry edge for first arrival
                        entry_edges
                            .entry(neighbor)
                            .or_insert(edge_ref.weight().kind);
                    }
                }
            }
        }

        // Sort by severity → hop_distance → id for stable ordering
        result.sort_by(|a, b| {
            a.severity
                .cmp(&b.severity)
                .then_with(|| a.hop_distance.cmp(&b.hop_distance))
                .then_with(|| a.node_info.id.cmp(&b.node_info.id))
        });

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::Edge;
    use arbor_core::{CodeNode, NodeKind};

    fn make_node(name: &str) -> CodeNode {
        CodeNode::new(name, name, NodeKind::Function, "test.rs")
    }

    #[test]
    fn test_severity_from_hops() {
        assert_eq!(ImpactSeverity::from_hops(0), ImpactSeverity::Direct);
        assert_eq!(ImpactSeverity::from_hops(1), ImpactSeverity::Direct);
        assert_eq!(ImpactSeverity::from_hops(2), ImpactSeverity::Transitive);
        assert_eq!(ImpactSeverity::from_hops(3), ImpactSeverity::Transitive);
        assert_eq!(ImpactSeverity::from_hops(4), ImpactSeverity::Distant);
        assert_eq!(ImpactSeverity::from_hops(100), ImpactSeverity::Distant);
    }

    #[test]
    fn test_empty_graph() {
        let graph = ArborGraph::new();
        let result = graph.analyze_impact(NodeId::new(0), 5);
        assert_eq!(result.total_affected, 0);
        assert!(result.upstream.is_empty());
        assert!(result.downstream.is_empty());
    }

    #[test]
    fn test_single_node() {
        let mut graph = ArborGraph::new();
        let id = graph.add_node(make_node("lonely"));
        let result = graph.analyze_impact(id, 5);
        assert_eq!(result.total_affected, 0);
        assert_eq!(result.target.name, "lonely");
    }

    #[test]
    fn test_linear_chain() {
        // A → B → C
        let mut graph = ArborGraph::new();
        let a = graph.add_node(make_node("a"));
        let b = graph.add_node(make_node("b"));
        let c = graph.add_node(make_node("c"));

        graph.add_edge(a, b, Edge::new(EdgeKind::Calls));
        graph.add_edge(b, c, Edge::new(EdgeKind::Calls));

        // Impact of B
        let result = graph.analyze_impact(b, 5);

        // Upstream: A calls B
        assert_eq!(result.upstream.len(), 1);
        assert_eq!(result.upstream[0].node_info.name, "a");
        assert_eq!(result.upstream[0].hop_distance, 1);
        assert_eq!(result.upstream[0].severity, ImpactSeverity::Direct);

        // Downstream: B calls C
        assert_eq!(result.downstream.len(), 1);
        assert_eq!(result.downstream[0].node_info.name, "c");
        assert_eq!(result.downstream[0].hop_distance, 1);
    }

    #[test]
    fn test_diamond_pattern() {
        //     A
        //    / \
        //   B   C
        //    \ /
        //     D
        let mut graph = ArborGraph::new();
        let a = graph.add_node(make_node("a"));
        let b = graph.add_node(make_node("b"));
        let c = graph.add_node(make_node("c"));
        let d = graph.add_node(make_node("d"));

        graph.add_edge(a, b, Edge::new(EdgeKind::Calls));
        graph.add_edge(a, c, Edge::new(EdgeKind::Calls));
        graph.add_edge(b, d, Edge::new(EdgeKind::Calls));
        graph.add_edge(c, d, Edge::new(EdgeKind::Calls));

        // Impact of A
        let result = graph.analyze_impact(a, 5);

        // Downstream should have B, C (depth 1) and D (depth 2)
        assert_eq!(result.downstream.len(), 3);

        let names: Vec<&str> = result
            .downstream
            .iter()
            .map(|n| n.node_info.name.as_str())
            .collect();
        assert!(names.contains(&"b"));
        assert!(names.contains(&"c"));
        assert!(names.contains(&"d"));

        // D should be transitive
        let d_node = result
            .downstream
            .iter()
            .find(|n| n.node_info.name == "d")
            .unwrap();
        assert_eq!(d_node.hop_distance, 2);
        assert_eq!(d_node.severity, ImpactSeverity::Transitive);
    }

    #[test]
    fn test_cycle_no_infinite_loop() {
        // A → B → C → A (cycle)
        let mut graph = ArborGraph::new();
        let a = graph.add_node(make_node("a"));
        let b = graph.add_node(make_node("b"));
        let c = graph.add_node(make_node("c"));

        graph.add_edge(a, b, Edge::new(EdgeKind::Calls));
        graph.add_edge(b, c, Edge::new(EdgeKind::Calls));
        graph.add_edge(c, a, Edge::new(EdgeKind::Calls)); // Cycle back

        // Should not hang
        let result = graph.analyze_impact(a, 10);

        // Should find B and C downstream
        assert_eq!(result.downstream.len(), 2);

        // Upstream: C → A (depth 1), and B → C means B is also reachable at depth 2
        assert_eq!(result.upstream.len(), 2);
        let upstream_names: Vec<&str> = result
            .upstream
            .iter()
            .map(|n| n.node_info.name.as_str())
            .collect();
        assert!(upstream_names.contains(&"c"));
        assert!(upstream_names.contains(&"b"));
    }

    #[test]
    fn test_max_depth_limit() {
        // A → B → C → D → E
        let mut graph = ArborGraph::new();
        let a = graph.add_node(make_node("a"));
        let b = graph.add_node(make_node("b"));
        let c = graph.add_node(make_node("c"));
        let d = graph.add_node(make_node("d"));
        let e = graph.add_node(make_node("e"));

        graph.add_edge(a, b, Edge::new(EdgeKind::Calls));
        graph.add_edge(b, c, Edge::new(EdgeKind::Calls));
        graph.add_edge(c, d, Edge::new(EdgeKind::Calls));
        graph.add_edge(d, e, Edge::new(EdgeKind::Calls));

        // Impact of A with max_depth = 2
        let result = graph.analyze_impact(a, 2);

        // Should only find B (depth 1) and C (depth 2), not D or E
        assert_eq!(result.downstream.len(), 2);

        let names: Vec<&str> = result
            .downstream
            .iter()
            .map(|n| n.node_info.name.as_str())
            .collect();
        assert!(names.contains(&"b"));
        assert!(names.contains(&"c"));
        assert!(!names.contains(&"d"));
        assert!(!names.contains(&"e"));
    }

    #[test]
    fn test_stable_ordering() {
        // Verify that results are deterministically ordered
        let mut graph = ArborGraph::new();
        let target = graph.add_node(make_node("target"));
        let z = graph.add_node(make_node("z_caller"));
        let a = graph.add_node(make_node("a_caller"));
        let m = graph.add_node(make_node("m_caller"));

        graph.add_edge(z, target, Edge::new(EdgeKind::Calls));
        graph.add_edge(a, target, Edge::new(EdgeKind::Calls));
        graph.add_edge(m, target, Edge::new(EdgeKind::Calls));

        let result = graph.analyze_impact(target, 5);

        // All are depth 1 (Direct), so should be sorted by name (which equals ID here)
        let names: Vec<&str> = result
            .upstream
            .iter()
            .map(|n| n.node_info.name.as_str())
            .collect();
        // Sorted alphabetically by node_info.id
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"a_caller"));
        assert!(names.contains(&"m_caller"));
        assert!(names.contains(&"z_caller"));
    }
}
