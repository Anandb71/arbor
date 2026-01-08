//! Core graph data structure.
//!
//! The ArborGraph wraps petgraph and adds indexes for fast lookups.
//! It's the central data structure that everything else works with.

use crate::edge::{Edge, EdgeKind, GraphEdge};
use arbor_core::CodeNode;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef; // For edge_references
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
    pub(crate) graph: DiGraph<CodeNode, Edge>,

    /// Maps string IDs to graph node indexes.
    id_index: HashMap<String, NodeId>,

    /// Maps node names to node IDs (for search).
    name_index: HashMap<String, Vec<NodeId>>,

    /// Maps file paths to node IDs (for incremental updates).
    file_index: HashMap<String, Vec<NodeId>>,

    /// Centrality scores for ranking.
    centrality: HashMap<NodeId, f64>,
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
            graph: DiGraph::new(),
            id_index: HashMap::new(),
            name_index: HashMap::new(),
            file_index: HashMap::new(),
            centrality: HashMap::new(),
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
        self.name_index.entry(name).or_default().push(index);
        self.file_index.entry(file).or_default().push(index);

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
    pub fn search(&self, query: &str) -> Vec<&CodeNode> {
        let query_lower = query.to_lowercase();
        self.graph
            .node_weights()
            .filter(|node| node.name.to_lowercase().contains(&query_lower))
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
                    if let Some(name_list) = self.name_index.get_mut(&node.name) {
                        name_list.retain(|&idx| idx != index);
                    }
                    // Remove from id index
                    self.id_index.remove(&node.id);
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
        self.graph
            .edge_references()
            .map(|edge_ref| {
                let source = self
                    .graph
                    .node_weight(edge_ref.source())
                    .unwrap()
                    .id
                    .clone();
                let target = self
                    .graph
                    .node_weight(edge_ref.target())
                    .unwrap()
                    .id
                    .clone();
                let weight = edge_ref.weight(); // &Edge
                GraphEdge {
                    source,
                    target,
                    kind: weight.kind,
                }
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
