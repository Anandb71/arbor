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

    /// Percentile-rank centrality, comparable across repositories.
    centrality: HashMap<NodeId, f64>,

    /// Raw PageRank mass, kept so a recompute can warm-start from it.
    #[serde(default)]
    centrality_raw: HashMap<NodeId, f64>,

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
            centrality_raw: HashMap::new(),
            search_index: SearchIndex::new(),
        }
    }

    /// Rebuilds the search index from existing graph nodes.
    /// Call after deserialization since search_index is not serialized.
    pub fn rebuild_search_index(&mut self) {
        self.search_index = SearchIndex::new();
        for index in self.graph.node_indices() {
            if let Some(node) = self.graph.node_weight(index) {
                self.search_index.insert(&node.name, index);
                index_node_documentation(&mut self.search_index, node, index);
            }
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
        if let Some(node) = self.graph.node_weight(index) {
            index_node_documentation(&mut self.search_index, node, index);
        }

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

    /// Resolves a symbol name to candidate nodes, best match first.
    ///
    /// Names collide constantly — `getPath` may be a utility function *and* a
    /// method on four unrelated classes. Callers that took
    /// `find_by_name(..).first()` got whichever file happened to be parsed
    /// first and then answered as though it were the only candidate; on a real
    /// codebase that reported a heavily-used function as unreachable dead code.
    ///
    /// Ranking is by graph degree, then centrality, then file path — so the
    /// pick is meaningful and does not depend on parse order. An exact node-id
    /// match short-circuits to a single result.
    pub fn resolve_symbol_ranked(&self, name: &str) -> Vec<NodeId> {
        if let Some(idx) = self.get_index(name) {
            return vec![idx];
        }

        let mut candidates: Vec<NodeId> = self
            .find_by_name(name)
            .iter()
            .filter_map(|n| self.get_index(&n.id))
            .collect();

        candidates.sort_by(|&a, &b| {
            let degree = |i: NodeId| self.get_callers(i).len() + self.get_callees(i).len();
            degree(b)
                .cmp(&degree(a))
                .then_with(|| {
                    self.centrality(b)
                        .partial_cmp(&self.centrality(a))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| {
                    let file = |i: NodeId| self.get(i).map(|n| n.file.clone()).unwrap_or_default();
                    file(a).cmp(&file(b))
                })
        });

        candidates
    }

    /// The single node a user most likely meant by `name`.
    ///
    /// See [`resolve_symbol_ranked`](Self::resolve_symbol_ranked) for how ties
    /// are broken.
    pub fn resolve_symbol(&self, name: &str) -> Option<NodeId> {
        self.resolve_symbol_ranked(name).into_iter().next()
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

    /// Searches for nodes whose name literally contains the query.
    ///
    /// Substring matching only: `search("login")` will not find
    /// `get_authenticated`. Use [`search_ranked`](Self::search_ranked) when the
    /// caller is asking about a concept rather than spelling a name.
    pub fn search(&self, query: &str) -> Vec<&CodeNode> {
        self.search_index
            .search(query)
            .iter()
            .filter_map(|id| self.graph.node_weight(*id))
            .collect()
    }

    /// Searches names, identifier tokens, related concepts, and documentation.
    ///
    /// Returns hits ordered strongest-first, each labelled with *how* it
    /// matched — so a caller can present an exact hit and a concept guess
    /// differently instead of blending them into one opaque ranking.
    ///
    /// ```ignore
    /// // Finds `get_authenticated`, `verifyJwt`, `hashPassword`, …
    /// for (node, hit) in graph.search_ranked("login") {
    ///     println!("{} ({})", node.name, hit.kind.label());
    /// }
    /// ```
    pub fn search_ranked(&self, query: &str) -> Vec<(&CodeNode, crate::search_index::SearchHit)> {
        self.search_index
            .search_ranked(query)
            .into_iter()
            .filter_map(|hit| self.graph.node_weight(hit.id).map(|node| (node, hit)))
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

    /// Centrality as a percentile rank in `[0.0, 1.0]`.
    ///
    /// `0.9` means "more central than 90% of the nodes in this repository",
    /// and carries that meaning in every repository — so a threshold written
    /// against it behaves the same on a god-object monolith and a flat service.
    /// For the underlying PageRank mass see [`centrality_raw`](Self::centrality_raw).
    pub fn centrality(&self, index: NodeId) -> f64 {
        self.centrality.get(&index).copied().unwrap_or(0.0)
    }

    /// Raw PageRank mass for a node. Sums to ~1.0 across the graph.
    pub fn centrality_raw(&self, index: NodeId) -> f64 {
        self.centrality_raw.get(&index).copied().unwrap_or(0.0)
    }

    /// Iterates every edge weight, for density and confidence reporting.
    pub fn edge_weights(&self) -> impl Iterator<Item = &Edge> {
        self.graph.edge_weights()
    }

    /// Counts edges at or above [`Edge::CONFIDENT_THRESHOLD`].
    ///
    /// The gap between this and [`edge_count`](Self::edge_count) is how much of
    /// the graph rests on inference rather than proof — worth surfacing in any
    /// report that claims to describe what a change reaches.
    pub fn confident_edge_count(&self) -> usize {
        self.graph
            .edge_weights()
            .filter(|edge| edge.is_confident())
            .count()
    }

    /// Sets percentile centrality scores.
    ///
    /// Prefer [`set_centrality_scores`](Self::set_centrality_scores), which
    /// also stores the raw scores a warm start needs.
    pub fn set_centrality(&mut self, scores: HashMap<NodeId, f64>) {
        self.centrality = scores;
    }

    /// Stores both the raw and percentile forms from a computation.
    pub fn set_centrality_scores(&mut self, scores: crate::ranking::CentralityScores) {
        let (raw, percentile) = scores.into_parts();
        self.centrality_raw = raw;
        self.centrality = percentile;
    }

    /// Raw score map, for warm-starting a recompute.
    ///
    /// Falls back to the percentile map when raw scores are absent — a graph
    /// deserialized from a cache written before raw scores were stored. The
    /// warm-start rescale in [`crate::ranking`] tolerates any scalar multiple
    /// of the fixed point, so this degrades convergence speed, not correctness.
    pub fn centrality_map(&self) -> &HashMap<NodeId, f64> {
        if self.centrality_raw.is_empty() {
            &self.centrality
        } else {
            &self.centrality_raw
        }
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

    /// Returns all nodes detected as production entry points.
    pub fn list_entry_points(&self) -> Vec<&CodeNode> {
        use crate::heuristics::HeuristicsMatcher;
        self.graph
            .node_weights()
            .filter(|n| HeuristicsMatcher::is_likely_entry_point(n))
            .collect()
    }

    /// Returns all nodes in a file and the call edges between them.
    /// Edges returned as (caller_name, callee_name, edge_kind_debug_str) triples.
    pub fn nodes_in_file_with_edges(
        &self,
        file: &str,
    ) -> (Vec<&CodeNode>, Vec<(String, String, String)>) {
        // Exact key first; fall back to a separator-insensitive match.
        //
        // `file_index` is keyed on the path exactly as the parser recorded it.
        // Callers build lookup paths by joining a project root with a
        // forward-slash relative path, which on Windows yields a mixed
        // `C:\root\src/lib.rs` that never equals the stored `C:\root\src\lib.rs`.
        let node_ids: std::collections::HashSet<NodeId> = match self.file_index.get(file) {
            Some(ids) => ids.iter().copied().collect(),
            None => {
                let wanted = normalize_separators(file);
                self.file_index
                    .iter()
                    .filter(|(stored, _)| normalize_separators(stored) == wanted)
                    .flat_map(|(_, ids)| ids.iter().copied())
                    .collect()
            }
        };

        let nodes: Vec<&CodeNode> = node_ids
            .iter()
            .filter_map(|&id| self.graph.node_weight(id))
            .collect();

        let mut edges = Vec::new();
        for &from in &node_ids {
            for edge_ref in self
                .graph
                .edges_directed(from, petgraph::Direction::Outgoing)
            {
                let to = edge_ref.target();
                if node_ids.contains(&to) {
                    if let (Some(from_node), Some(to_node)) =
                        (self.graph.node_weight(from), self.graph.node_weight(to))
                    {
                        edges.push((
                            from_node.name.clone(),
                            to_node.name.clone(),
                            format!("{:?}", edge_ref.weight().kind),
                        ));
                    }
                }
            }
        }
        (nodes, edges)
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

#[cfg(test)]
mod new_query_tests {
    use super::*;
    use crate::edge::{Edge, EdgeKind};
    use arbor_core::{CodeNode, NodeKind};

    fn make_node(name: &str, kind: NodeKind, file: &str) -> CodeNode {
        CodeNode::new(name, format!("{}::{}", file, name), kind, file)
    }

    #[test]
    fn test_list_entry_points_returns_main() {
        let mut g = ArborGraph::new();
        g.add_node(make_node("main", NodeKind::Function, "src/main.rs"));
        g.add_node(make_node("helper", NodeKind::Function, "src/util.rs"));
        let eps = g.list_entry_points();
        assert!(
            eps.iter().any(|n| n.name == "main"),
            "main must be an entry point"
        );
        assert!(
            !eps.iter().any(|n| n.name == "helper"),
            "helper must not be an entry point"
        );
    }

    #[test]
    fn test_nodes_in_file_with_edges_returns_edges() {
        let mut g = ArborGraph::new();
        let a = g.add_node(make_node("foo", NodeKind::Function, "src/a.rs"));
        let b = g.add_node(make_node("bar", NodeKind::Function, "src/a.rs"));
        let _c = g.add_node(make_node("baz", NodeKind::Function, "src/b.rs"));
        g.add_edge(a, b, Edge::new(EdgeKind::Calls));
        let (nodes, edges) = g.nodes_in_file_with_edges("src/a.rs");
        assert_eq!(nodes.len(), 2);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].0, "foo");
        assert_eq!(edges[0].1, "bar");
    }

    #[test]
    fn nodes_in_file_lookup_is_separator_insensitive() {
        // Callers join a project root with a forward-slash relative path,
        // which on Windows yields `C:\root\src/lib.rs` while the parser
        // recorded `C:\root\src\lib.rs`. Exact-key lookup missed, and
        // `file-graph` reported "No symbols found" for a file full of symbols.
        let mut g = ArborGraph::new();
        g.add_node(make_node(
            "helper",
            NodeKind::Function,
            r"C:/root/src\lib.rs",
        ));

        let (backslash, _) = g.nodes_in_file_with_edges(r"C:/root/src\lib.rs");
        assert_eq!(backslash.len(), 1, "exact key must still work");

        let (forward, _) = g.nodes_in_file_with_edges("C:/root/src/lib.rs");
        assert_eq!(forward.len(), 1, "separator spelling must not matter");
    }

    #[test]
    fn test_nodes_in_file_with_edges_excludes_cross_file_edges() {
        use crate::edge::{Edge, EdgeKind};
        let mut g = ArborGraph::new();
        let a = g.add_node(make_node("foo", NodeKind::Function, "src/a.rs"));
        let c = g.add_node(make_node("baz", NodeKind::Function, "src/b.rs"));
        // Edge from a.rs to b.rs — should NOT appear in get_file_graph for a.rs
        g.add_edge(a, c, Edge::new(EdgeKind::Calls));
        let (nodes, edges) = g.nodes_in_file_with_edges("src/a.rs");
        assert_eq!(nodes.len(), 1); // only foo
        assert_eq!(edges.len(), 0); // cross-file edge excluded
    }
}

/// Feeds a node's supporting text into the search index.
///
/// Docstrings, signatures, and qualified names are already parsed into every
/// `CodeNode` and were previously ignored at search time — so a function
/// documented as "validates the user's login credentials" was unreachable by a
/// search for "login". The file path is included too: `src/auth/session.ts`
/// says as much about a symbol as its name often does.
fn index_node_documentation(index: &mut SearchIndex, node: &CodeNode, id: NodeId) {
    if let Some(doc) = &node.docstring {
        index.insert_documentation(doc, id);
    }
    if let Some(sig) = &node.signature {
        index.insert_documentation(sig, id);
    }
    index.insert_documentation(&node.file, id);
    if node.qualified_name != node.name {
        index.insert_documentation(&node.qualified_name, id);
    }
}

/// Normalizes path separators for comparison.
///
/// Windows accepts both `/` and `\`, so the same file can be spelled either
/// way depending on whether the path came from a directory walk or from
/// joining a user-supplied relative path.
fn normalize_separators(path: &str) -> String {
    path.replace('\\', "/")
}
