//! Graph builder for constructing the code graph from parsed nodes.
//!
//! The builder takes CodeNodes and resolves their references into
//! actual graph edges.

use crate::edge::{Edge, EdgeKind};
use crate::graph::{ArborGraph, NodeId};
use crate::symbol_table::SymbolTable;
use arbor_core::CodeNode;
use std::collections::HashMap;
use std::path::PathBuf;

/// Builds an ArborGraph from parsed code nodes.
///
/// The builder handles the two-pass process:
/// 1. Add all nodes to the graph
/// 2. Resolve references into edges (including cross-file)
pub struct GraphBuilder {
    graph: ArborGraph,
    /// Maps qualified names to node IDs for edge resolution.
    symbol_table: SymbolTable,
    /// Legacy map for simple name resolution (within same file)
    name_to_id: HashMap<String, String>,
}

impl Default for GraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphBuilder {
    /// Creates a new builder.
    pub fn new() -> Self {
        Self {
            graph: ArborGraph::new(),
            symbol_table: SymbolTable::new(),
            name_to_id: HashMap::new(),
        }
    }

    /// Adds nodes from a file to the graph.
    ///
    /// Call this for each parsed file, then call `resolve_edges`
    /// when all files are added.
    pub fn add_nodes(&mut self, nodes: Vec<CodeNode>) {
        for node in nodes {
            let id_str = node.id.clone();
            let name = node.name.clone();
            let qualified = node.qualified_name.clone();
            let file = PathBuf::from(&node.file);

            let node_idx = self.graph.add_node(node);

            // Populate Symbol Table
            if !qualified.is_empty() {
                self.symbol_table
                    .insert(qualified.clone(), node_idx, file.clone());
            }

            self.name_to_id.insert(name.clone(), id_str.clone());
            self.name_to_id.insert(qualified, id_str);
        }
    }

    /// Resolves references into actual graph edges.
    ///
    /// This is the second pass after all nodes are added. It looks up
    /// reference names and creates edges where targets exist.
    pub fn resolve_edges(&mut self) {
        // Collect all the edge additions first to avoid borrow issues
        let mut edges_to_add = Vec::new();

        // Collect indices to avoid borrowing self.graph during iteration
        let node_indices: Vec<NodeId> = self.graph.node_indexes().collect();

        for from_idx in node_indices {
            // Get references by cloning to release borrow on graph
            let references = {
                let node = self.graph.get(from_idx).unwrap();
                node.references.clone()
            };

            for reference in references {
                let mut found = false;

                // 1. Try resolving via Symbol Table (FQN)
                if let Some(to_idx) = self.symbol_table.resolve(&reference) {
                    if from_idx != to_idx {
                        edges_to_add.push((from_idx, to_idx, reference.clone()));
                        found = true;
                    }
                }

                if found {
                    continue;
                }

                // 2. Fallback to legacy ID map
                if let Some(to_id_str) = self.name_to_id.get(&reference) {
                    if let Some(to_idx) = self.graph.get_index(to_id_str) {
                        if from_idx != to_idx {
                            edges_to_add.push((from_idx, to_idx, reference.clone()));
                        }
                    }
                }
            }
        }

        // Now add the edges
        for (from_id, to_id, _ref_name) in edges_to_add {
            self.graph
                .add_edge(from_id, to_id, Edge::new(EdgeKind::Calls));
        }
    }

    /// Finishes building and returns the graph.
    pub fn build(mut self) -> ArborGraph {
        self.resolve_edges();
        self.graph
    }

    /// Builds without resolving edges (for incremental updates).
    pub fn build_without_resolve(self) -> ArborGraph {
        self.graph
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arbor_core::NodeKind;

    #[test]
    fn test_builder_adds_nodes() {
        let mut builder = GraphBuilder::new();

        let node1 = CodeNode::new("foo", "foo", NodeKind::Function, "test.rs");
        let node2 = CodeNode::new("bar", "bar", NodeKind::Function, "test.rs");

        builder.add_nodes(vec![node1, node2]);
        let graph = builder.build();

        assert_eq!(graph.node_count(), 2);
    }

    #[test]
    fn test_builder_resolves_edges() {
        let mut builder = GraphBuilder::new();

        let caller = CodeNode::new("caller", "caller", NodeKind::Function, "test.rs")
            .with_references(vec!["callee".to_string()]);
        let callee = CodeNode::new("callee", "callee", NodeKind::Function, "test.rs");

        builder.add_nodes(vec![caller, callee]);
        let graph = builder.build();

        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn test_cross_file_resolution() {
        let mut builder = GraphBuilder::new();

        // File A: Calls "pkg.Utils.helper"
        let caller = CodeNode::new("main", "main", NodeKind::Function, "main.rs")
            .with_references(vec!["pkg.Utils.helper".to_string()]);

        // File B: Defines "pkg.Utils.helper"
        let mut callee = CodeNode::new("helper", "helper", NodeKind::Method, "utils.rs");
        callee.qualified_name = "pkg.Utils.helper".to_string();

        builder.add_nodes(vec![caller]);
        builder.add_nodes(vec![callee]);

        let graph = builder.build();

        assert_eq!(graph.node_count(), 2);
        assert_eq!(
            graph.edge_count(),
            1,
            "Should resolve cross-file edge via FQN"
        );
    }
}
