//! Graph builder for constructing the code graph from parsed nodes.
//!
//! Two-pass construction:
//!   1. Add all nodes — populates symbol table and import map
//!   2. Resolve edges — uses import context to create accurate edges

use crate::edge::{Edge, EdgeKind};
use crate::graph::{ArborGraph, NodeId};
use crate::symbol_table::SymbolTable;
use arbor_core::{CodeNode, NodeKind};
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::warn;

/// Builds an ArborGraph from parsed code nodes.
pub struct GraphBuilder {
    graph: ArborGraph,
    symbol_table: SymbolTable,
    name_to_id: HashMap<String, String>,

    /// Per-file map of locally-bound name → source module specifier.
    /// Built from Import nodes that carry their imported names in `references`.
    ///
    /// Example:
    ///   `import { validate } from '@babel/types'`
    ///   → import_map["file.ts"]["validate"] = "@babel/types"
    ///
    /// Used during edge resolution to verify that a direct call like `validate()`
    /// is indeed an intentional import, not a same-name coincidence.
    import_map: HashMap<String, HashMap<String, String>>,

    /// Namespace import aliases: file → alias → source module.
    ///
    /// Example:
    ///   `import * as types from '@babel/types'`
    ///   → namespace_imports["file.ts"]["types"] = "@babel/types"
    ///
    /// Used to resolve calls like `types.validate()` — though since we now DROP
    /// those calls at parse time, this is reserved for future use when we add
    /// a richer call-site representation.
    namespace_imports: HashMap<String, HashMap<String, String>>,
}

impl Default for GraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphBuilder {
    pub fn new() -> Self {
        Self {
            graph: ArborGraph::new(),
            symbol_table: SymbolTable::new(),
            name_to_id: HashMap::new(),
            import_map: HashMap::new(),
            namespace_imports: HashMap::new(),
        }
    }

    /// Adds nodes from a parsed file to the graph.
    ///
    /// This pass does two things:
    ///   - Adds real code entities (functions, classes, etc.) to the graph and symbol table
    ///   - Processes Import nodes to build the per-file import map
    ///
    /// Call this for all files before calling `build()`.
    pub fn add_nodes(&mut self, nodes: Vec<CodeNode>) {
        for node in nodes {
            // Import nodes carry the import map — process them but don't add to graph
            // (they are not code entities we want in centrality analysis)
            if node.kind == NodeKind::Import {
                let file = node.file.clone();
                let module = node.name.clone();

                for imported_name in &node.references {
                    if let Some(alias) = imported_name.strip_prefix("*as:") {
                        // `import * as alias from 'module'`
                        self.namespace_imports
                            .entry(file.clone())
                            .or_default()
                            .insert(alias.to_string(), module.clone());
                    } else {
                        // `import { name } from 'module'` or `import DefaultName from 'module'`
                        self.import_map
                            .entry(file.clone())
                            .or_default()
                            .insert(imported_name.clone(), module.clone());
                    }
                }
                // Import nodes are intentionally NOT added to the graph.
                // They caused misleading centrality scores (e.g. "psycopg.types.range [Import]
                // 330 callers") because every call to a symbol named after the import path
                // was attributed to the import node itself.
                continue;
            }

            let id_str = node.id.clone();
            let name = node.name.clone();
            let qualified = node.qualified_name.clone();
            let file = PathBuf::from(&node.file);

            let node_idx = self.graph.add_node(node);

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
    /// Resolution order for each reference `R` from file `F`:
    ///   1. Exact FQN match in symbol table
    ///   2. Context-aware suffix match (prefers same directory, avoids ambiguity)
    ///   3. Import-validated match — if R is in F's import map AND a match was found
    ///      in step 2 for a different file, we skip it to avoid wrong-module edges
    ///
    /// References that cannot be resolved are silently dropped (they are external/stdlib
    /// symbols with no definition in this repository).
    pub fn resolve_edges(&mut self) {
        let mut edges_to_add: Vec<(NodeId, NodeId, String)> = Vec::new();

        let node_indices: Vec<NodeId> = self.graph.node_indexes().collect();

        for from_idx in node_indices {
            let (references, from_file) = {
                let node = self.graph.get(from_idx).unwrap();
                (node.references.clone(), PathBuf::from(&node.file))
            };

            let from_file_str = from_file.to_string_lossy().to_string();

            for reference in references {
                // 1. Exact FQN match
                if let Some(to_idx) = self.symbol_table.resolve(&reference) {
                    if from_idx != to_idx {
                        edges_to_add.push((from_idx, to_idx, reference.clone()));
                    }
                    continue;
                }

                // 2. Context-aware suffix match
                if let Some(to_idx) = self
                    .symbol_table
                    .resolve_with_context(&reference, &from_file)
                {
                    if from_idx == to_idx {
                        continue;
                    }

                    // 3. Import-validation filter
                    //
                    // If this file has an explicit import map AND the reference is NOT
                    // in it, the suffix match may have found a wrong-file coincidence.
                    // Only apply this filter when the file has import data (not all parsers
                    // provide it yet) and the reference is a simple name (no dots).
                    //
                    // Skip if: the file has imports, the name is NOT imported, and the
                    // matched node is in a completely different part of the tree.
                    // This prevents `validate()` in file X from linking to `validate` in
                    // an unrelated module when `validate` is not imported.
                    if let Some(file_imports) = self.import_map.get(&from_file_str) {
                        if !file_imports.is_empty()
                            && !reference.contains('.')
                            && !file_imports.contains_key(&reference)
                        {
                            // Not imported explicitly — only allow if in same file or same dir
                            let to_node = self.graph.get(to_idx).unwrap();
                            let to_file = PathBuf::from(&to_node.file);
                            let same_file = to_file == from_file;
                            let same_dir = to_file.parent() == from_file.parent();
                            if !same_file && !same_dir {
                                warn!(
                                    "Skipping unimported cross-module reference '{}' in {} → {}",
                                    reference,
                                    from_file.display(),
                                    to_file.display()
                                );
                                continue;
                            }
                        }
                    }

                    edges_to_add.push((from_idx, to_idx, reference.clone()));
                    continue;
                }

                // Unresolved: external/stdlib symbol — silently drop (expected)
                #[cfg(debug_assertions)]
                warn!(
                    "Unresolved reference '{}' in {} (likely external/stdlib)",
                    reference,
                    from_file.display()
                );
            }
        }

        for (from_id, to_id, _) in edges_to_add {
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
        let caller = CodeNode::new("main", "main", NodeKind::Function, "main.rs")
            .with_references(vec!["pkg.Utils.helper".to_string()]);
        let mut callee = CodeNode::new("helper", "helper", NodeKind::Method, "utils.rs");
        callee.qualified_name = "pkg.Utils.helper".to_string();
        builder.add_nodes(vec![caller]);
        builder.add_nodes(vec![callee]);
        let graph = builder.build();
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1, "Should resolve cross-file edge via FQN");
    }

    #[test]
    fn test_unresolved_references_no_false_edges() {
        let mut builder = GraphBuilder::new();
        let node = CodeNode::new("caller", "caller", NodeKind::Function, "a.rs")
            .with_references(vec!["nonexistent_function".to_string()]);
        builder.add_nodes(vec![node]);
        let graph = builder.build();
        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.edge_count(), 0, "Unresolved references must not create edges");
    }

    #[test]
    fn test_import_nodes_not_added_to_graph() {
        let mut builder = GraphBuilder::new();
        let import_node =
            CodeNode::new("./utils", "./utils", NodeKind::Import, "main.ts")
                .with_references(vec!["validate".to_string()]);
        let func = CodeNode::new("main", "main", NodeKind::Function, "main.ts");
        builder.add_nodes(vec![import_node, func]);
        let graph = builder.build();
        // Only the function should be in the graph, not the import node
        assert_eq!(graph.node_count(), 1);
    }

    #[test]
    fn test_import_map_built_correctly() {
        let mut builder = GraphBuilder::new();
        let import_node =
            CodeNode::new("@babel/types", "@babel/types", NodeKind::Import, "file.ts")
                .with_references(vec!["validate".to_string(), "clone".to_string()]);
        builder.add_nodes(vec![import_node]);
        assert_eq!(
            builder.import_map.get("file.ts").and_then(|m| m.get("validate")),
            Some(&"@babel/types".to_string())
        );
        assert_eq!(
            builder.import_map.get("file.ts").and_then(|m| m.get("clone")),
            Some(&"@babel/types".to_string())
        );
    }

    #[test]
    fn test_namespace_import_map() {
        let mut builder = GraphBuilder::new();
        let import_node =
            CodeNode::new("@babel/types", "@babel/types", NodeKind::Import, "file.ts")
                .with_references(vec!["*as:types".to_string()]);
        builder.add_nodes(vec![import_node]);
        assert_eq!(
            builder
                .namespace_imports
                .get("file.ts")
                .and_then(|m| m.get("types")),
            Some(&"@babel/types".to_string())
        );
    }

    #[test]
    fn test_build_empty_graph() {
        let builder = GraphBuilder::new();
        let graph = builder.build();
        assert_eq!(graph.node_count(), 0);
        assert_eq!(graph.edge_count(), 0);
    }
}
