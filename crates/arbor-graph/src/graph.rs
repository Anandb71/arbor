//! Core graph data structure.
//!
//! The ArborGraph wraps petgraph and adds indexes for fast lookups.
//! It's the central data structure that everything else works with.

use crate::edge::{Edge, EdgeKind, GraphEdge};
use crate::search_index::SearchIndex;
use arbor_core::CodeNode;
use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use petgraph::visit::{EdgeRef, IntoEdgeReferences}; // For edge_references
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Unique identifier for a node in the graph.
pub type NodeId = NodeIndex;

/// The code relationship graph.
///
/// This is the heart of Arbor. It stores all code entities as nodes
/// and their relationships as edges, with indexes for fast access.
#[derive(Debug, Serialize, Deserialize)]
pub struct ArborGraph {
    /// The underlying petgraph graph.
    pub(crate) graph: StableDiGraph<CodeNode, Edge>,

    /// Maps string IDs to graph node indexes.
    id_index: HashMap<String, NodeId>,

    /// Maps node names to node IDs (for search).
    name_index: HashMap<String, Vec<NodeId>>,

    /// Maps file paths to node IDs (for incremental updates).
    file_index: HashMap<String, Vec<NodeId>>,

    /// Centrality scores for ranking.
    centrality: HashMap<NodeId, f64>,

    /// Search index for fast substring queries.
    #[serde(skip)]
    search_index: SearchIndex,
}

impl Default for ArborGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl ArborGraph {
    /// Creates a new empty graph.
    pub fn new() -> Self {
        Self {
            graph: StableDiGraph::new(),
            id_index: HashMap::new(),
            name_index: HashMap::new(),
            file_index: HashMap::new(),
            centrality: HashMap::new(),
            search_index: SearchIndex::new(),
        }
    }

    /// Adds a code node to the graph.
    ///
    /// Returns the node's index for adding edges later.
    pub fn add_node(&mut self, node: CodeNode) -> NodeId {
        let id = node.id.clone();
        let name = node.name.clone();
        let file = node.file.clone();

        let index = self.graph.add_node(node);

        // Update indexes
        self.id_index.insert(id, index);
        self.name_index.entry(name.clone()).or_default().push(index);
        self.file_index.entry(file).or_default().push(index);
        self.search_index.insert(&name, index);

        index
    }

    /// Adds an edge between two nodes.
    pub fn add_edge(&mut self, from: NodeId, to: NodeId, edge: Edge) {
        self.graph.add_edge(from, to, edge);
    }

    /// Gets a node by its string ID.
    pub fn get_by_id(&self, id: &str) -> Option<&CodeNode> {
        let index = self.id_index.get(id)?;
        self.graph.node_weight(*index)
    }

    /// Gets a node by its graph index.
    pub fn get(&self, index: NodeId) -> Option<&CodeNode> {
        self.graph.node_weight(index)
    }

    /// Finds all nodes with a given name.
    pub fn find_by_name(&self, name: &str) -> Vec<&CodeNode> {
        self.name_index
            .get(name)
            .map(|indexes| {
                indexes
                    .iter()
                    .filter_map(|idx| self.graph.node_weight(*idx))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Finds all nodes in a file.
    pub fn find_by_file(&self, file: &str) -> Vec<&CodeNode> {
        self.file_index
            .get(file)
            .map(|indexes| {
                indexes
                    .iter()
                    .filter_map(|idx| self.graph.node_weight(*idx))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Searches for nodes whose name contains the query.
    ///
    /// Uses the search index for fast O(k) lookups where k is the number of matches,
    /// instead of O(n) linear scan over all nodes.
    pub fn search(&self, query: &str) -> Vec<&CodeNode> {
        self.search_index
            .search(query)
            .iter()
            .filter_map(|id| self.graph.node_weight(*id))
            .collect()
    }

    /// Gets nodes that call the given node.
    pub fn get_callers(&self, index: NodeId) -> Vec<&CodeNode> {
        self.graph
            .neighbors_directed(index, petgraph::Direction::Incoming)
            .filter_map(|idx| {
                // Check if the edge is a call
                let edge_idx = self.graph.find_edge(idx, index)?;
                let edge = self.graph.edge_weight(edge_idx)?;
                if edge.kind == EdgeKind::Calls {
                    self.graph.node_weight(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Gets nodes that this node calls.
    pub fn get_callees(&self, index: NodeId) -> Vec<&CodeNode> {
        self.graph
            .neighbors_directed(index, petgraph::Direction::Outgoing)
            .filter_map(|idx| {
                let edge_idx = self.graph.find_edge(index, idx)?;
                let edge = self.graph.edge_weight(edge_idx)?;
                if edge.kind == EdgeKind::Calls {
                    self.graph.node_weight(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Gets all nodes that depend on the given node (directly or transitively).
    pub fn get_dependents(&self, index: NodeId, max_depth: usize) -> Vec<(NodeId, usize)> {
        let mut result = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut queue = vec![(index, 0usize)];

        while let Some((current, depth)) = queue.pop() {
            if depth > max_depth || visited.contains(&current) {
                continue;
            }
            visited.insert(current);

            if current != index {
                result.push((current, depth));
            }

            // Get incoming edges (callers)
            for neighbor in self
                .graph
                .neighbors_directed(current, petgraph::Direction::Incoming)
            {
                if !visited.contains(&neighbor) {
                    queue.push((neighbor, depth + 1));
                }
            }
        }

        result
    }

    /// Removes all nodes from a file. Used for incremental updates.
    pub fn remove_file(&mut self, file: &str) {
        if let Some(indexes) = self.file_index.remove(file) {
            for index in indexes {
                if let Some(node) = self.graph.node_weight(index) {
                    // Remove from name index
                    let name = node.name.clone();
                    if let Some(name_list) = self.name_index.get_mut(&name) {
                        name_list.retain(|&idx| idx != index);
                    }
                    // Remove from id index
                    self.id_index.remove(&node.id);
                    // Remove from search index
                    self.search_index.remove(&name, index);
                }
                self.graph.remove_node(index);
            }
        }
    }

    /// Gets the centrality score for a node.
    pub fn centrality(&self, index: NodeId) -> f64 {
        self.centrality.get(&index).copied().unwrap_or(0.0)
    }

    /// Sets centrality scores (called after computation).
    pub fn set_centrality(&mut self, scores: HashMap<NodeId, f64>) {
        self.centrality = scores;
    }

    /// Returns the number of nodes.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Returns the number of edges.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Iterates over all nodes.
    pub fn nodes(&self) -> impl Iterator<Item = &CodeNode> {
        self.graph.node_weights()
    }

    /// Iterates over all edges.
    pub fn edges(&self) -> impl Iterator<Item = &Edge> {
        self.graph.edge_weights()
    }

    /// Returns all edges with source and target IDs for export.
    pub fn export_edges(&self) -> Vec<GraphEdge> {
        (&self.graph)
            .edge_references()
            .filter_map(|edge_ref| {
                let source = self.graph.node_weight(edge_ref.source())?.id.clone();
                let target = self.graph.node_weight(edge_ref.target())?.id.clone();
                let weight = edge_ref.weight(); // &Edge
                Some(GraphEdge {
                    source,
                    target,
                    kind: weight.kind,
                })
            })
            .collect()
    }

    /// Iterates over all node indexes.
    pub fn node_indexes(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.graph.node_indices()
    }

    /// Finds the shortest path between two nodes.
    pub fn find_path(&self, from: NodeId, to: NodeId) -> Option<Vec<&CodeNode>> {
        let path_indices = petgraph::algo::astar(
            &self.graph,
            from,
            |finish| finish == to,
            |_| 1, // weight of 1 for all edges (BFS-like)
            |_| 0, // heuristic
        )?;

        Some(
            path_indices
                .1
                .into_iter()
                .filter_map(|idx| self.graph.node_weight(idx))
                .collect(),
        )
    }

    /// Gets the node index for a string ID.
    pub fn get_index(&self, id: &str) -> Option<NodeId> {
        self.id_index.get(id).copied()
    }
}

/// Graph statistics for the info endpoint.
#[derive(Debug, Serialize, Deserialize)]
pub struct GraphStats {
    pub node_count: usize,
    pub edge_count: usize,
    pub files: usize,
}

impl ArborGraph {
    /// Returns graph statistics.
    pub fn stats(&self) -> GraphStats {
        GraphStats {
            node_count: self.node_count(),
            edge_count: self.edge_count(),
            files: self.file_index.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::{Edge, EdgeKind};
    use arbor_core::{CodeNode, NodeKind};

    fn make_node(name: &str, file: &str) -> CodeNode {
        CodeNode::new(name, name, NodeKind::Function, file)
    }

    #[test]
    fn test_graph_new_is_empty() {
        let g = ArborGraph::new();
        assert_eq!(g.node_count(), 0);
        assert_eq!(g.edge_count(), 0);
        assert!(g.nodes().next().is_none());
    }

    #[test]
    fn test_graph_add_and_get_node() {
        let mut g = ArborGraph::new();
        let node = make_node("foo", "main.rs");
        let id = g.add_node(node.clone());
        assert_eq!(g.node_count(), 1);

        let got = g.get(id).unwrap();
        assert_eq!(got.name, "foo");
    }

    #[test]
    fn test_graph_find_by_name() {
        let mut g = ArborGraph::new();
        g.add_node(make_node("alpha", "a.rs"));
        g.add_node(make_node("beta", "b.rs"));

        let found = g.find_by_name("alpha");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "alpha");

        let not_found = g.find_by_name("gamma");
        assert!(not_found.is_empty());
    }

    #[test]
    fn test_graph_find_by_file() {
        let mut g = ArborGraph::new();
        g.add_node(make_node("foo", "main.rs"));
        g.add_node(make_node("bar", "main.rs"));
        g.add_node(make_node("baz", "other.rs"));

        let main_nodes = g.find_by_file("main.rs");
        assert_eq!(main_nodes.len(), 2);

        let other_nodes = g.find_by_file("other.rs");
        assert_eq!(other_nodes.len(), 1);

        let empty = g.find_by_file("nonexistent.rs");
        assert!(empty.is_empty());
    }

    #[test]
    fn test_graph_search_substring() {
        let mut g = ArborGraph::new();
        g.add_node(make_node("validate_user", "a.rs"));
        g.add_node(make_node("validate_email", "b.rs"));
        g.add_node(make_node("send_email", "c.rs"));

        let results = g.search("validate");
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|n| n.name == "validate_user"));
        assert!(results.iter().any(|n| n.name == "validate_email"));
    }

    #[test]
    fn test_graph_callers_callees() {
        let mut g = ArborGraph::new();
        let a = g.add_node(make_node("caller", "a.rs"));
        let b = g.add_node(make_node("callee", "b.rs"));
        g.add_edge(a, b, Edge::new(EdgeKind::Calls));

        let callees = g.get_callees(a);
        assert_eq!(callees.len(), 1);
        assert_eq!(callees[0].name, "callee");

        let callers = g.get_callers(b);
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].name, "caller");

        // No callers/callees for disconnected nodes
        assert!(g.get_callers(a).is_empty());
        assert!(g.get_callees(b).is_empty());
    }

    #[test]
    fn test_graph_get_dependents() {
        // a -> b -> c
        let mut g = ArborGraph::new();
        let a = g.add_node(make_node("a", "a.rs"));
        let b = g.add_node(make_node("b", "b.rs"));
        let c = g.add_node(make_node("c", "c.rs"));
        g.add_edge(a, b, Edge::new(EdgeKind::Calls));
        g.add_edge(b, c, Edge::new(EdgeKind::Calls));

        // Dependents of c at depth 2 should include a and b
        let deps = g.get_dependents(c, 2);
        assert!(deps.iter().any(|(idx, _)| g.get(*idx).unwrap().name == "b"));
        assert!(deps.iter().any(|(idx, _)| g.get(*idx).unwrap().name == "a"));
    }

    #[test]
    fn test_graph_remove_file_cleanup() {
        let mut g = ArborGraph::new();
        g.add_node(make_node("foo", "remove_me.rs"));
        g.add_node(make_node("bar", "remove_me.rs"));
        g.add_node(make_node("keep", "keep.rs"));

        assert_eq!(g.node_count(), 3);

        g.remove_file("remove_me.rs");

        // Nodes from removed file are gone
        assert!(g.find_by_name("foo").is_empty());
        assert!(g.find_by_name("bar").is_empty());
        // Node from other file remains
        assert_eq!(g.find_by_name("keep").len(), 1);
        assert!(g.find_by_file("remove_me.rs").is_empty());
    }

    #[test]
    fn test_graph_find_path() {
        // a -> b -> c
        let mut g = ArborGraph::new();
        let a = g.add_node(make_node("start", "a.rs"));
        let b = g.add_node(make_node("middle", "b.rs"));
        let c = g.add_node(make_node("end", "c.rs"));
        g.add_edge(a, b, Edge::new(EdgeKind::Calls));
        g.add_edge(b, c, Edge::new(EdgeKind::Calls));

        let path = g.find_path(a, c).unwrap();
        assert_eq!(path.len(), 3);
        assert_eq!(path[0].name, "start");
        assert_eq!(path[1].name, "middle");
        assert_eq!(path[2].name, "end");
    }

    #[test]
    fn test_graph_find_path_no_connection() {
        let mut g = ArborGraph::new();
        let a = g.add_node(make_node("island_a", "a.rs"));
        let b = g.add_node(make_node("island_b", "b.rs"));

        // No edges → no path
        assert!(g.find_path(a, b).is_none());
    }

    #[test]
    fn test_graph_export_edges() {
        let mut g = ArborGraph::new();
        let a = g.add_node(make_node("a", "a.rs"));
        let b = g.add_node(make_node("b", "b.rs"));
        g.add_edge(a, b, Edge::new(EdgeKind::Calls));

        let exported = g.export_edges();
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].kind, EdgeKind::Calls);
    }

    #[test]
    fn test_graph_stats() {
        let mut g = ArborGraph::new();
        g.add_node(make_node("a", "x.rs"));
        g.add_node(make_node("b", "y.rs"));

        let stats = g.stats();
        assert_eq!(stats.node_count, 2);
        assert_eq!(stats.edge_count, 0);
        assert_eq!(stats.files, 2);
    }

    #[test]
    fn test_graph_get_index_and_get_by_id() {
        let mut g = ArborGraph::new();
        let node = make_node("lookup_me", "test.rs");
        let node_id_str = node.id.clone();
        let idx = g.add_node(node);

        assert_eq!(g.get_index(&node_id_str), Some(idx));
        assert!(g.get_by_id(&node_id_str).is_some());
        assert!(g.get_index("nonexistent").is_none());
        assert!(g.get_by_id("nonexistent").is_none());
    }

    #[test]
    fn test_graph_centrality_default_zero() {
        let mut g = ArborGraph::new();
        let idx = g.add_node(make_node("a", "a.rs"));
        assert_eq!(g.centrality(idx), 0.0);
    }

    #[test]
    fn test_graph_set_centrality() {
        let mut g = ArborGraph::new();
        let idx = g.add_node(make_node("a", "a.rs"));

        let mut scores = HashMap::new();
        scores.insert(idx, 0.75);
        g.set_centrality(scores);

        assert!((g.centrality(idx) - 0.75).abs() < f64::EPSILON);
    }
}
