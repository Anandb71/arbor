//! Centrality ranking for code nodes.
//!
//! We use a production-aware PageRank variant: callers from test files
//! contribute 10x less weight than production callers, so utility functions
//! called heavily by tests don't false-inflate centrality scores.

use crate::edge::EdgeKind;
use crate::graph::{ArborGraph, NodeId};
use arbor_core::NodeKind;
use petgraph::visit::{EdgeRef, IntoEdgeReferences};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Iteration stops early once no node's score moves more than this between
/// rounds. Tight enough that early exit is indistinguishable from running
/// the full iteration budget.
const CONVERGENCE_EPSILON: f64 = 1e-9;

/// Centrality scores in two forms.
///
/// # Why two
///
/// Raw mass is the propagation state. Sinks and the test-file weight drop
/// some of it, so a graph does not sum to 1. An individual value still
/// shrinks as the repository grows and means nothing on its own. The previous
/// implementation divided every score by the maximum, which fixed the range but
/// produced values that are not comparable between repositories: the top node
/// is 1.0 *by construction* whether it has four callers or four hundred, and a
/// threshold like `0.6` therefore means "60% as central as whatever the biggest
/// thing here happens to be." In a repo with one god object nothing ever
/// cleared it; in a flat repo almost everything did. Worse, adding a single new
/// hub rescaled every other node in the graph.
///
/// [`percentile`](Self::percentile) is the comparable form — `0.6` means "more
/// central than 60% of this repository" everywhere — and is what thresholds
/// should use. [`raw`](Self::raw) is the true fixed point, kept because
/// warm-start recomputation needs it.
#[derive(Debug, Default, Clone)]
pub struct CentralityScores {
    raw: HashMap<NodeId, f64>,
    percentile: HashMap<NodeId, f64>,
}

impl CentralityScores {
    /// Percentile rank in `[0.0, 1.0]` — comparable across repositories.
    pub fn get(&self, id: NodeId) -> f64 {
        self.percentile.get(&id).copied().unwrap_or(0.0)
    }

    /// Raw propagation mass for this node. The graph sum is at most 1.
    pub fn get_raw(&self, id: NodeId) -> f64 {
        self.raw.get(&id).copied().unwrap_or(0.0)
    }

    /// Percentile map, for display and thresholding.
    pub fn into_map(self) -> HashMap<NodeId, f64> {
        self.percentile
    }

    /// Raw map, for warm-starting a later recompute.
    pub fn into_raw_map(self) -> HashMap<NodeId, f64> {
        self.raw
    }

    /// Both maps as `(raw, percentile)`.
    pub fn into_parts(self) -> (HashMap<NodeId, f64>, HashMap<NodeId, f64>) {
        (self.raw, self.percentile)
    }

    /// Builds percentile ranks from raw scores.
    ///
    /// A node's percentile is the fraction of nodes scoring strictly below it,
    /// `rank = i / (n - 1)`, so tied nodes share a rank and the ordering is
    /// total and deterministic. This is the v2.6.0 contract: `0.6` means
    /// strictly above 60% of this repository, which is what `arbor agent review`
    /// and `arbor agent guard` threshold against.
    fn from_raw(nodes: Vec<NodeId>, scores: Vec<f64>) -> Self {
        let n = scores.len();
        let mut percentile = vec![0.0f64; n];

        if n == 1 {
            percentile[0] = 1.0;
        } else if n > 1 {
            let mut order: Vec<usize> = (0..n).collect();
            // Tie-break on index so the order never depends on hash iteration.
            order.sort_by(|&a, &b| {
                scores[a]
                    .partial_cmp(&scores[b])
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.cmp(&b))
            });

            let denom = (n - 1) as f64;
            let mut i = 0;
            while i < n {
                let mut j = i;
                while j + 1 < n && scores[order[j + 1]] == scores[order[i]] {
                    j += 1;
                }
                let rank = i as f64 / denom;
                for slot in &order[i..=j] {
                    percentile[*slot] = rank;
                }
                i = j + 1;
            }
        }

        Self {
            raw: nodes.iter().copied().zip(scores).collect(),
            percentile: nodes.into_iter().zip(percentile).collect(),
        }
    }
}

/// Returns true if this file path is a test/spec/fixture file.
/// Callers from test files get de-weighted 10x so test utilities don't
/// false-inflate their centrality scores vs. production callers.
fn is_test_file(file: &str) -> bool {
    let lower = file.to_lowercase();
    lower.contains("/test")
        || lower.contains("\\test")
        || lower.contains("/spec")
        || lower.contains("\\spec")
        || lower.contains("__test__")
        || lower.contains("_test.")
        || lower.contains(".test.")
        || lower.contains(".spec.")
        || lower.contains("/fixture")
        || lower.contains("/mock")
        || lower.contains("/stub")
        || lower.contains("/fake")
        || lower.ends_with("_test.go")
        || lower.ends_with("_test.py")
        || lower.ends_with("_test.rs")
        || lower.ends_with("test.ts")
        || lower.ends_with("test.js")
}

/// Iterative Kosaraju on the call graph. Returns the component id of each node.
///
/// Iterative on purpose: the 500-node ring in the torture fixture is exactly
/// the kind of chain that would overflow a recursive DFS.
fn scc_ids(out_edges: &[Vec<u32>], in_edges: &[Vec<u32>]) -> Vec<usize> {
    let n = out_edges.len();
    let mut visited = vec![false; n];
    let mut order = Vec::with_capacity(n);

    for start in 0..n {
        if visited[start] {
            continue;
        }
        let mut stack = vec![(start, 0usize)];
        visited[start] = true;
        while let Some((u, i)) = stack.pop() {
            if i < out_edges[u].len() {
                stack.push((u, i + 1));
                let v = out_edges[u][i] as usize;
                if !visited[v] {
                    visited[v] = true;
                    stack.push((v, 0));
                }
            } else {
                order.push(u);
            }
        }
    }

    visited.fill(false);
    let mut scc_id = vec![0usize; n];
    let mut scc_count = 0usize;
    for &start in order.iter().rev() {
        if visited[start] {
            continue;
        }
        let mut stack = vec![start];
        visited[start] = true;
        while let Some(u) = stack.pop() {
            scc_id[u] = scc_count;
            for &pred in &in_edges[u] {
                let v = pred as usize;
                if !visited[v] {
                    visited[v] = true;
                    stack.push(v);
                }
            }
        }
        scc_count += 1;
    }
    scc_id
}

/// Call-graph condensation: one supernode per SCC, edges only between components.
///
/// Every cycle is collapsed, whether or not it can call out. Internal edges
/// do not carry mass, so a ring cannot trap the walk and a cycle that calls a
/// helper scales with that external edge instead of circulating at damping 0.85.
struct CondensedCalls {
    /// Dense component id of each node.
    scc: Vec<usize>,
    /// Members of each component, in node-index order.
    members: Vec<Vec<u32>>,
    /// External in-flows: `(source component, factor)` per component.
    incoming: Vec<Vec<(u32, f64)>>,
    /// Sum of outgoing factors. Zero when the component calls nothing outside itself.
    out_factor: Vec<f64>,
    /// Largest caller-weight among members that have an external call.
    /// Production components emit `1.0`; a component that only leaves through
    /// a test file emits `0.1`, matching the singleton de-weight.
    emission: Vec<f64>,
}

fn condense_calls(
    out_edges: &[Vec<u32>],
    in_edges: &[Vec<u32>],
    weights: &[f64],
) -> CondensedCalls {
    let scc = scc_ids(out_edges, in_edges);
    let ncomp = scc.iter().copied().max().map(|m| m + 1).unwrap_or(0);
    let mut members = vec![Vec::new(); ncomp];
    for (node, &comp) in scc.iter().enumerate() {
        members[comp].push(node as u32);
    }

    // BTreeMap, not HashMap: a component that calls two targets sums those
    // factors here, and HashMap iteration order changes the float sum across
    // processes. Rankings are compared to 12 decimal places.
    let mut buckets: Vec<BTreeMap<u32, f64>> = vec![BTreeMap::new(); ncomp];
    let mut emission = vec![0.0f64; ncomp];
    for (source, targets) in out_edges.iter().enumerate() {
        let comp = scc[source];
        let external: Vec<usize> = targets
            .iter()
            .map(|target| *target as usize)
            .filter(|&target| scc[target] != comp)
            .collect();
        if external.is_empty() {
            continue;
        }
        emission[comp] = emission[comp].max(weights[source]);
        let share = weights[source] / external.len() as f64;
        for target in external {
            *buckets[comp].entry(scc[target] as u32).or_insert(0.0) += share;
        }
    }

    let mut out_factor = vec![0.0f64; ncomp];
    let mut incoming = vec![Vec::new(); ncomp];
    for (comp, bucket) in buckets.iter().enumerate() {
        let sum: f64 = bucket.values().sum();
        out_factor[comp] = sum;
        for (&target, &factor) in bucket {
            incoming[target as usize].push((comp as u32, factor));
        }
    }

    CondensedCalls {
        scc,
        members,
        incoming,
        out_factor,
        emission,
    }
}

/// Computes production-aware centrality scores for all nodes in the graph.
///
/// Uses a modified PageRank where:
/// 1. Nodes initialize with equal score
/// 2. Strongly connected components of the call graph are condensed. PageRank
///    runs on that DAG, then each component's mass is shared across its members
///    so a cycle cannot trap the walk — including cycles that call out to a helper
/// 3. Each iteration distributes component mass along external call edges,
///    damped at 0.85 (0.15 teleports uniformly, per node)
/// 4. Callers from test/spec/fixture files contribute 10x less weight
///    — prevents test utilities from appearing more central than production code
/// 5. Raw scores are converted to percentile ranks (`i / (n - 1)`) for
///    cross-repo comparability (see [`CentralityScores`])
///
/// # Arguments
///
/// * `graph` - The graph to analyze
/// * `iterations` - Maximum power-iteration rounds (10-20 is usually enough).
///   The loop stops early once no member score moves by more than `1e-9`.
///   A smaller budget is honored; it is not raised internally.
/// * `damping` - Damping factor (0.85 is standard)
pub fn compute_centrality(graph: &ArborGraph, iterations: usize, damping: f64) -> CentralityScores {
    compute_centrality_warm(graph, iterations, damping, None)
}

/// Like [`compute_centrality`], but seeds the iteration from a previous score
/// map (e.g. [`ArborGraph::centrality_map`]) instead of a uniform start.
///
/// The iteration is a damped affine contraction, so it converges to the same
/// fixed point from any starting vector — warm-starting only changes how many
/// rounds it takes. After a small graph patch the previous scores are already
/// near the fixed point and the loop exits after one or two rounds, which is
/// what makes watcher-driven recomputes cheap.
pub fn compute_centrality_warm(
    graph: &ArborGraph,
    iterations: usize,
    damping: f64,
    previous: Option<&HashMap<NodeId, f64>>,
) -> CentralityScores {
    let node_count = graph.node_count();
    if node_count == 0 {
        return CentralityScores::default();
    }

    // Flatten the (possibly holey, StableGraph) node set into dense positions
    // so the hot loop runs over Vecs instead of HashMaps.
    let nodes: Vec<NodeId> = graph.node_indexes().collect();
    let n = nodes.len();
    let pos: HashMap<NodeId, usize> = nodes.iter().enumerate().map(|(i, &id)| (id, i)).collect();

    // Test callers contribute 10% weight — they inflate utility functions
    // but don't represent real production blast radius
    let weights: Vec<f64> = nodes
        .iter()
        .map(|&id| match graph.get(id) {
            Some(node) if is_test_file(&node.file) => 0.1,
            _ => 1.0,
        })
        .collect();

    // Calls only. Import and containment edges are not part of the walk.
    let mut out_edges: Vec<Vec<u32>> = vec![Vec::new(); n];
    let mut in_edges: Vec<Vec<u32>> = vec![Vec::new(); n];
    for edge in graph.graph.edge_references() {
        if edge.weight().kind != EdgeKind::Calls {
            continue;
        }
        let (Some(&source), Some(&target)) = (pos.get(&edge.source()), pos.get(&edge.target()))
        else {
            continue;
        };
        out_edges[source].push(target as u32);
        in_edges[target].push(source as u32);
    }

    let condensed = condense_calls(&out_edges, &in_edges, &weights);
    let ncomp = condensed.members.len();
    let sizes: Vec<f64> = condensed.members.iter().map(|m| m.len() as f64).collect();

    let initial_score = 1.0 / n as f64;
    let base = (1.0 - damping) / n as f64;
    // Component mass update. A singleton reproduces the historical per-node
    // equation `base + d * Σ weight * score / out_degree`. A multi-node SCC
    // emits its whole mass along external edges (split by those edges' factors)
    // and splits the result evenly, so both sides of `a <-> b` stay tied.
    let step = |mass: &[f64]| -> Vec<f64> {
        (0..ncomp)
            .map(|comp| {
                let inflow: f64 = condensed.incoming[comp]
                    .iter()
                    .map(|&(source, factor)| {
                        let source = source as usize;
                        let denom = condensed.out_factor[source];
                        if denom <= 0.0 {
                            0.0
                        } else {
                            mass[source] * condensed.emission[source] * factor / denom
                        }
                    })
                    .sum();
                sizes[comp] * base + damping * inflow
            })
            .collect()
    };

    let mut mass: Vec<f64> = match previous {
        Some(prev) if !prev.is_empty() => {
            let mut warm = vec![0.0f64; ncomp];
            for (index, id) in nodes.iter().enumerate() {
                warm[condensed.scc[index]] += prev.get(id).copied().unwrap_or(initial_score);
            }
            // Stored scores may be any scalar multiple c of the iteration's
            // fixed point. For raw scores (what `centrality_map` now returns)
            // c ≈ 1 and this is a no-op; the rescale is kept so a caller that
            // hands us normalized scores still converges. For v ≈ c·x*, summing
            // the fixed-point equation gives c = 1 − (f(v) − Σv) / (1 − d).
            let sum_v: f64 = warm.iter().sum();
            let applied = step(&warm);
            let f_v: f64 = applied.iter().sum();
            let teleport = (1.0 - damping).max(f64::EPSILON);
            let c = 1.0 - (f_v - sum_v) / teleport;
            if c.is_finite() && c > f64::EPSILON {
                for score in warm.iter_mut() {
                    *score /= c;
                }
            }
            warm
        }
        _ => sizes.iter().map(|size| size * initial_score).collect(),
    };

    for _ in 0..iterations {
        let applied = step(&mass);
        let mut max_delta = 0.0f64;
        for comp in 0..ncomp {
            let old = mass[comp] / sizes[comp];
            let new = applied[comp] / sizes[comp];
            max_delta = max_delta.max((new - old).abs());
        }
        mass = applied;
        if max_delta < CONVERGENCE_EPSILON {
            break;
        }
    }

    let mut scores = vec![0.0f64; n];
    for (comp, members) in condensed.members.iter().enumerate() {
        let share = mass[comp] / sizes[comp];
        for &member in members {
            scores[member as usize] = share;
        }
    }

    CentralityScores::from_raw(nodes, scores)
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

/// Bundled vendor trees and minified assets that should not rank as hotspots.
///
/// Used when a graph was indexed before those paths were excluded, so a stale
/// cache still hides them from the hotspot table.
pub fn is_noise_path(file: &str) -> bool {
    let lower = normalize_path(file);
    let with_boundary = format!("/{lower}/");

    lower.ends_with(".min.js")
        || lower.ends_with(".min.css")
        || lower.contains(".chunk.")
        || lower.contains(".bundle.")
        || with_boundary.contains("/vendor/")
        || with_boundary.contains("/dist/")
        || with_boundary.contains("/build/")
        || with_boundary.contains("/generated/")
        || {
            let filename = Path::new(&lower)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            let parts: Vec<&str> = filename.split('.').collect();
            parts.len() >= 3
                && parts[1].len() >= 8
                && parts[1].chars().all(|c| c.is_ascii_hexdigit())
        }
}

/// Computes percentile centrality when the graph has nodes and no complete score map.
///
/// Returns `true` when a computation was stored. Callers that persist the graph
/// should write the cache only in that case.
pub fn ensure_centrality(graph: &mut ArborGraph, iterations: usize, damping: f64) -> bool {
    if graph.node_count() == 0 || graph.has_centrality_scores() {
        return false;
    }
    let scores = compute_centrality(graph, iterations, damping);
    graph.set_centrality_scores(scores);
    true
}

/// Highest-centrality symbols that participate in the graph and are not vendor noise.
///
/// Disconnected symbols are omitted. A percentile of `0.0` is kept: that is the
/// bottom rank after a real computation, not a missing score.
pub fn top_hotspots(graph: &ArborGraph, limit: usize) -> Vec<(NodeId, f64)> {
    let mut candidates = Vec::new();

    for idx in graph.node_indexes() {
        let Some(node) = graph.get(idx) else {
            continue;
        };
        if is_noise_path(&node.file) {
            continue;
        }
        if matches!(
            node.kind,
            NodeKind::Import | NodeKind::Export | NodeKind::Module
        ) {
            continue;
        }
        let degree = graph.get_callers(idx).len() + graph.get_callees(idx).len();
        if degree == 0 {
            continue;
        }
        candidates.push((
            idx,
            graph.centrality(idx),
            degree,
            node.file.clone(),
            node.name.clone(),
        ));
    }

    candidates.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.3.cmp(&b.3))
            .then_with(|| a.4.cmp(&b.4))
    });

    candidates
        .into_iter()
        .take(limit)
        .map(|(idx, centrality, _, _, _)| (idx, centrality))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::{Edge, EdgeKind};
    use arbor_core::{CodeNode, NodeKind};

    #[test]
    fn test_centrality_empty_graph() {
        let graph = ArborGraph::new();
        let scores = compute_centrality(&graph, 10, 0.85);
        assert!(scores.percentile.is_empty());
    }

    #[test]
    fn test_centrality_single_node() {
        let mut graph = ArborGraph::new();
        let node = CodeNode::new("foo", "foo", NodeKind::Function, "test.rs");
        graph.add_node(node);

        let scores = compute_centrality(&graph, 10, 0.85);
        assert_eq!(scores.percentile.len(), 1);
    }

    #[test]
    fn test_centrality_popular_node_ranks_higher() {
        let mut graph = ArborGraph::new();

        // Create a "popular" function called by many others
        let popular = CodeNode::new("popular", "popular", NodeKind::Function, "test.rs");
        let popular_idx = graph.add_node(popular);

        // Create callers
        for i in 0..5 {
            let caller = CodeNode::new(
                format!("caller{}", i),
                format!("caller{}", i),
                NodeKind::Function,
                "test.rs",
            );
            let caller_idx = graph.add_node(caller);
            graph.add_edge(caller_idx, popular_idx, Edge::new(EdgeKind::Calls));
        }

        let scores = compute_centrality(&graph, 20, 0.85);

        // The popular node should have the highest score
        let popular_score = scores.get(popular_idx);
        assert!(popular_score > 0.5, "Popular node should rank high");
    }

    #[test]
    fn test_centrality_test_callers_deweighted() {
        let mut graph = ArborGraph::new();

        let prod_target = CodeNode::new("prod_target", "prod_target", NodeKind::Function, "a.rs");
        let prod_target_idx = graph.add_node(prod_target);
        let test_target = CodeNode::new("test_target", "test_target", NodeKind::Function, "a.rs");
        let test_target_idx = graph.add_node(test_target);

        // One production caller vs one test caller, same shape otherwise.
        let prod_caller = CodeNode::new("prod_caller", "prod_caller", NodeKind::Function, "b.rs");
        let prod_caller_idx = graph.add_node(prod_caller);
        graph.add_edge(prod_caller_idx, prod_target_idx, Edge::new(EdgeKind::Calls));

        let test_caller = CodeNode::new(
            "test_caller",
            "test_caller",
            NodeKind::Function,
            "tests/b_test.rs",
        );
        let test_caller_idx = graph.add_node(test_caller);
        graph.add_edge(test_caller_idx, test_target_idx, Edge::new(EdgeKind::Calls));

        let scores = compute_centrality(&graph, 20, 0.85);
        assert!(
            scores.get(prod_target_idx) > scores.get(test_target_idx),
            "production callers must outweigh test callers"
        );
    }

    #[test]
    fn test_warm_start_matches_cold_start() {
        let mut graph = ArborGraph::new();
        let hub = graph.add_node(CodeNode::new("hub", "hub", NodeKind::Function, "hub.rs"));
        let mut previous = std::collections::HashMap::new();
        for i in 0..10 {
            let caller = graph.add_node(CodeNode::new(
                format!("c{}", i),
                format!("c{}", i),
                NodeKind::Function,
                "c.rs",
            ));
            graph.add_edge(caller, hub, Edge::new(EdgeKind::Calls));
            previous.insert(caller, 0.3);
        }
        previous.insert(hub, 1.0);

        let cold = compute_centrality(&graph, 50, 0.85);
        let warm = compute_centrality_warm(&graph, 50, 0.85, Some(&previous));

        for idx in graph.node_indexes() {
            assert!(
                (cold.get(idx) - warm.get(idx)).abs() < 1e-6,
                "warm start must converge to the same fixed point"
            );
        }
    }

    #[test]
    fn test_only_calls_edges_contribute() {
        let mut graph = ArborGraph::new();
        let a = graph.add_node(CodeNode::new("a", "a", NodeKind::Function, "a.rs"));
        let b = graph.add_node(CodeNode::new("b", "b", NodeKind::Function, "b.rs"));
        let c = graph.add_node(CodeNode::new("c", "c", NodeKind::Function, "c.rs"));

        // b is called; c is only imported — c must not gain call centrality.
        graph.add_edge(a, b, Edge::new(EdgeKind::Calls));
        graph.add_edge(a, c, Edge::new(EdgeKind::Imports));

        let scores = compute_centrality(&graph, 20, 0.85);
        assert!(
            scores.get(b) > scores.get(c),
            "import edges must not count as calls"
        );
    }

    #[test]
    fn extends_edges_do_not_change_percentile() {
        let mut graph = ArborGraph::new();
        let caller = graph.add_node(CodeNode::new(
            "caller",
            "caller",
            NodeKind::Function,
            "a.py",
        ));
        let callee = graph.add_node(CodeNode::new(
            "callee",
            "callee",
            NodeKind::Function,
            "a.py",
        ));
        let base = graph.add_node(CodeNode::new("Base", "Base", NodeKind::Class, "a.py"));
        let sub = graph.add_node(CodeNode::new("Sub", "Sub", NodeKind::Class, "a.py"));
        graph.add_edge(caller, callee, Edge::new(EdgeKind::Calls));

        let before = compute_centrality(&graph, 20, 0.85);
        graph.add_edge(sub, base, Edge::new(EdgeKind::Extends));
        let after = compute_centrality(&graph, 20, 0.85);

        for idx in graph.node_indexes() {
            assert!(
                (before.get(idx) - after.get(idx)).abs() < 1e-12,
                "extends must not move the percentile"
            );
            assert!(
                (before.get_raw(idx) - after.get_raw(idx)).abs() < 1e-12,
                "extends must not move raw PageRank mass"
            );
        }
    }

    /// Builds a star: `spokes` callers all pointing at one hub.
    fn star(spokes: usize) -> (ArborGraph, NodeId) {
        let mut graph = ArborGraph::new();
        let hub = graph.add_node(CodeNode::new("hub", "hub", NodeKind::Function, "hub.rs"));
        for i in 0..spokes {
            let name = format!("s{i}");
            let s = graph.add_node(CodeNode::new(
                name.clone(),
                name,
                NodeKind::Function,
                format!("s{i}.rs"),
            ));
            graph.add_edge(s, hub, Edge::new(EdgeKind::Calls));
        }
        (graph, hub)
    }

    #[test]
    fn percentile_is_comparable_across_graph_shapes() {
        // The old max-normalization gave the hub exactly 1.0 in both graphs and
        // told you nothing about how it compared to its peers. Percentile still
        // ranks the hub top, but every other node now sits at a rank that means
        // the same thing in both repos.
        let (small, small_hub) = star(3);
        let (large, large_hub) = star(60);

        let small_scores = compute_centrality(&small, 20, 0.85);
        let large_scores = compute_centrality(&large, 20, 0.85);

        assert_eq!(small_scores.get(small_hub), 1.0);
        assert_eq!(large_scores.get(large_hub), 1.0);

        // Spokes are all tied at the bottom in both graphs.
        for (graph, scores) in [(&small, &small_scores), (&large, &large_scores)] {
            for idx in graph.node_indexes() {
                let is_hub = graph.get(idx).map(|n| n.name == "hub").unwrap_or(false);
                if !is_hub {
                    assert_eq!(scores.get(idx), 0.0, "tied spokes share the bottom rank");
                }
            }
        }
    }

    #[test]
    fn adding_a_hub_does_not_rescale_unrelated_nodes() {
        // Under max-normalization, introducing a bigger hub divided every other
        // node's score by a larger maximum, silently changing the reported
        // centrality — and therefore the risk level — of untouched code.
        let mut graph = ArborGraph::new();
        let a = graph.add_node(CodeNode::new("a", "a", NodeKind::Function, "a.rs"));
        let b = graph.add_node(CodeNode::new("b", "b", NodeKind::Function, "b.rs"));
        let c = graph.add_node(CodeNode::new("c", "c", NodeKind::Function, "c.rs"));
        graph.add_edge(a, b, Edge::new(EdgeKind::Calls));
        graph.add_edge(c, b, Edge::new(EdgeKind::Calls));

        let before = compute_centrality(&graph, 20, 0.85).get(b);

        // Add a far more connected hub elsewhere in the repo.
        let hub = graph.add_node(CodeNode::new("hub", "hub", NodeKind::Function, "hub.rs"));
        for i in 0..20 {
            let name = format!("x{i}");
            let x = graph.add_node(CodeNode::new(
                name.clone(),
                name,
                NodeKind::Function,
                format!("x{i}.rs"),
            ));
            graph.add_edge(x, hub, Edge::new(EdgeKind::Calls));
        }

        let after = compute_centrality(&graph, 20, 0.85).get(b);

        // b is still ranked above the leaf callers that make up the bulk of the
        // graph; it did not collapse toward zero just because a hub appeared.
        assert!(
            after > 0.5,
            "b should remain in the upper half, got {after} (was {before})"
        );
    }

    #[test]
    fn percentile_ties_are_stable_and_ordering_preserved() {
        let (graph, hub) = star(5);
        let scores = compute_centrality(&graph, 20, 0.85);

        // Recomputing must give identical values — no hash-order dependence.
        let again = compute_centrality(&graph, 20, 0.85);
        for idx in graph.node_indexes() {
            assert_eq!(scores.get(idx), again.get(idx));
            assert_eq!(scores.get_raw(idx), again.get_raw(idx));
        }

        // Raw ordering must agree with percentile ordering.
        for idx in graph.node_indexes() {
            if idx != hub {
                assert!(scores.get_raw(hub) > scores.get_raw(idx));
                assert!(scores.get(hub) > scores.get(idx));
            }
        }
    }

    #[test]
    fn single_node_graph_is_top_ranked() {
        let mut graph = ArborGraph::new();
        let only = graph.add_node(CodeNode::new("solo", "solo", NodeKind::Function, "a.rs"));
        let scores = compute_centrality(&graph, 20, 0.85);
        assert_eq!(scores.get(only), 1.0);
    }

    #[test]
    fn warm_start_from_raw_matches_cold_result() {
        let (graph, _) = star(30);
        let cold = compute_centrality(&graph, 20, 0.85);
        let warm = compute_centrality_warm(&graph, 20, 0.85, Some(&cold.clone().into_raw_map()));

        for idx in graph.node_indexes() {
            assert!(
                (cold.get_raw(idx) - warm.get_raw(idx)).abs() < 1e-9,
                "warm start must converge to the same fixed point"
            );
        }
    }

    #[test]
    fn noise_path_detects_vendor_and_minified_assets() {
        assert!(is_noise_path(
            "android/app/src/main/assets/instrument/vendor/maplibre-gl.js"
        ));
        assert!(is_noise_path("static/app.min.js"));
        assert!(is_noise_path("dist/main.d094b1b69ba24b63.js"));
        assert!(!is_noise_path("src/main/kotlin/MainActivity.kt"));
    }

    #[test]
    fn top_hotspots_keeps_connected_bottom_rank_and_drops_vendor() {
        let mut graph = ArborGraph::new();
        let hub = graph.add_node(CodeNode::new(
            "hub",
            "hub",
            NodeKind::Function,
            "src/hub.rs",
        ));
        let caller = graph.add_node(CodeNode::new(
            "caller",
            "caller",
            NodeKind::Function,
            "src/a.rs",
        ));
        let _vendor = graph.add_node(CodeNode::new(
            "el",
            "el",
            NodeKind::Function,
            "android/app/src/main/assets/vendor/app.js",
        ));
        let _orphan = graph.add_node(CodeNode::new(
            "orphan",
            "orphan",
            NodeKind::Function,
            "b.rs",
        ));
        graph.add_edge(caller, hub, Edge::new(EdgeKind::Calls));

        assert!(ensure_centrality(&mut graph, 20, 0.85));
        assert!(!ensure_centrality(&mut graph, 20, 0.85));
        assert_eq!(graph.centrality(caller), 0.0);

        let names: Vec<String> = top_hotspots(&graph, 10)
            .iter()
            .filter_map(|(idx, _)| graph.get(*idx).map(|node| node.name.clone()))
            .collect();
        assert!(names.contains(&"hub".to_string()));
        assert!(names.contains(&"caller".to_string()));
        assert!(!names.contains(&"el".to_string()));
        assert!(!names.contains(&"orphan".to_string()));
    }

    #[test]
    fn ensure_centrality_does_not_recompute_a_flat_graph() {
        let mut graph = ArborGraph::new();
        graph.add_node(CodeNode::new("a", "a", NodeKind::Function, "a.rs"));
        graph.add_node(CodeNode::new("b", "b", NodeKind::Function, "b.rs"));

        assert!(ensure_centrality(&mut graph, 20, 0.85));
        assert!(graph.has_centrality_scores());
        assert!(!ensure_centrality(&mut graph, 20, 0.85));
        assert!(top_hotspots(&graph, 10).is_empty());
    }

    fn fn_node(graph: &mut ArborGraph, name: &str, file: &str) -> NodeId {
        graph.add_node(CodeNode::new(name, name, NodeKind::Function, file))
    }

    fn link(graph: &mut ArborGraph, from: NodeId, to: NodeId) {
        graph.add_edge(from, to, Edge::new(EdgeKind::Calls));
    }

    /// Closed call ring: r0 → r1 → … → r{n-1} → r0.
    fn ring(n: usize) -> (ArborGraph, Vec<NodeId>) {
        let mut graph = ArborGraph::new();
        let mut nodes = Vec::with_capacity(n);
        for i in 0..n {
            let name = format!("r{i:03}");
            nodes.push(fn_node(&mut graph, &name, "evil/ring500.py"));
        }
        for i in 0..n {
            link(&mut graph, nodes[i], nodes[(i + 1) % n]);
        }
        (graph, nodes)
    }

    #[test]
    fn closed_ring500_stays_out_of_the_top_decile() {
        let (mut graph, ring_nodes) = ring(500);
        let hub = fn_node(&mut graph, "hub", "hub.rs");
        for i in 0..12 {
            let name = format!("c{i}");
            let caller = fn_node(&mut graph, &name, &format!("c{i}.rs"));
            link(&mut graph, caller, hub);
        }

        let scores = compute_centrality(&graph, 20, 0.85);
        let hub_rank = scores.get(hub);
        let mut in_top_decile = 0usize;
        for &idx in &ring_nodes {
            let rank = scores.get(idx);
            assert!(
                rank < hub_rank,
                "ring member ranked {rank} against hub {hub_rank}"
            );
            if rank > 0.90 {
                in_top_decile += 1;
            }
        }
        assert_eq!(
            in_top_decile, 0,
            "isolated 500-ring occupied the top decile"
        );
    }

    #[test]
    fn cycle_with_exit_keeps_both_members_above_dead_code() {
        // caller → a ↔ b, and a calls helper. The component can leave, so it
        // is not a sink, but a and b still share one rank. b must not collapse
        // to the orphan's score the way a sink-only edge deletion does.
        let mut graph = ArborGraph::new();
        let caller = fn_node(&mut graph, "caller", "caller.rs");
        let a = fn_node(&mut graph, "a", "cycle.rs");
        let b = fn_node(&mut graph, "b", "cycle.rs");
        let helper = fn_node(&mut graph, "helper", "helper.rs");
        let orphan = fn_node(&mut graph, "orphan", "orphan.rs");
        link(&mut graph, caller, a);
        link(&mut graph, a, b);
        link(&mut graph, b, a);
        link(&mut graph, a, helper);

        let scores = compute_centrality(&graph, 20, 0.85);
        assert!(
            (scores.get_raw(a) - scores.get_raw(b)).abs() < 1e-9,
            "cycle members share component mass, a={} b={}",
            scores.get_raw(a),
            scores.get_raw(b)
        );
        assert_eq!(scores.get(a), scores.get(b));
        assert!(
            scores.get_raw(b) > scores.get_raw(orphan),
            "b must stay above dead code"
        );
        assert!(scores.get(b) > scores.get(orphan));

        // The exit receives the component's mass, not just a's personal share.
        let n = graph.node_count() as f64;
        let base = (1.0 - 0.85) / n;
        let forwarded = base + 0.85 * (2.0 * scores.get_raw(a));
        assert!(
            (scores.get_raw(helper) - forwarded).abs() < 1e-8,
            "helper={} expected {forwarded} (full component mass)",
            scores.get_raw(helper)
        );
        let sum: f64 = graph.node_indexes().map(|idx| scores.get_raw(idx)).sum();
        assert!(sum <= 1.0 + 1e-6, "cycle created mass, sum={sum}");
        assert!(sum > 0.0);
    }

    #[test]
    fn circular_import_pair_shares_rank() {
        // Same shape as arbor-torture `app/utils/circular_a.py` and `circular_b.py`:
        // alpha calls beta, beta calls alpha.
        let parser = arbor_core::languages::get_parser("py").expect("python parser");
        let alpha_src = "\
def alpha(n: int) -> int:\n    \
    from app.utils.circular_b import beta\n    \
    if n <= 0:\n        \
        return 0\n    \
    return beta(n - 1) + 1\n";
        let beta_src = "\
def beta(n: int) -> int:\n    \
    from app.utils.circular_a import alpha\n    \
    if n <= 0:\n        \
        return 0\n    \
    return alpha(n - 1) + 1\n";
        let nodes_a =
            arbor_core::parse_source(alpha_src, "app/utils/circular_a.py", parser.as_ref())
                .expect("parse circular_a");
        let nodes_b =
            arbor_core::parse_source(beta_src, "app/utils/circular_b.py", parser.as_ref())
                .expect("parse circular_b");
        let mut builder = crate::builder::GraphBuilder::new();
        builder.add_nodes(nodes_a);
        builder.add_nodes(nodes_b);
        let graph = builder.build();

        let alpha_id = graph
            .find_by_name("alpha")
            .first()
            .expect("alpha indexed")
            .id
            .clone();
        let beta_id = graph
            .find_by_name("beta")
            .first()
            .expect("beta indexed")
            .id
            .clone();
        let alpha = graph.get_index(&alpha_id).expect("alpha id");
        let beta = graph.get_index(&beta_id).expect("beta id");
        assert!(
            !graph.get_callees(alpha).is_empty() && !graph.get_callees(beta).is_empty(),
            "circular_a/circular_b must resolve to a real call cycle"
        );

        let scores = compute_centrality(&graph, 20, 0.85);
        assert!(
            (scores.get_raw(alpha) - scores.get_raw(beta)).abs() < 1e-9,
            "alpha={} beta={}",
            scores.get_raw(alpha),
            scores.get_raw(beta)
        );
        assert_eq!(scores.get(alpha), scores.get(beta));
    }

    #[test]
    fn non_star_percentile_matches_v2_6_rank_formula() {
        // A chain is not a star. Percentile must be exactly i/(n-1) for the
        // strict raw order — not a log blend, which would move the middle nodes.
        let mut graph = ArborGraph::new();
        let mut nodes = Vec::new();
        let mut prev = fn_node(&mut graph, "n0", "chain.rs");
        nodes.push(prev);
        for i in 1..4 {
            let name = format!("n{i}");
            let cur = fn_node(&mut graph, &name, "chain.rs");
            link(&mut graph, prev, cur);
            nodes.push(cur);
            prev = cur;
        }

        let scores = compute_centrality(&graph, 20, 0.85);
        let mut order = nodes.clone();
        order.sort_by(|&a, &b| {
            scores
                .get_raw(a)
                .partial_cmp(&scores.get_raw(b))
                .unwrap()
                .then_with(|| a.index().cmp(&b.index()))
        });
        let denom = (order.len() - 1) as f64;
        for (i, idx) in order.iter().enumerate() {
            let expected = i as f64 / denom;
            assert_eq!(
                scores.get(*idx),
                expected,
                "node {i} percentile drifted from the v2.6.0 rank"
            );
        }
        assert!(scores.get_raw(order[0]) < scores.get_raw(order[1]));
        assert!(scores.get_raw(order[2]) < scores.get_raw(order[3]));
    }

    #[test]
    fn non_sink_cycle_forwards_mass_without_creating_it() {
        let mut graph = ArborGraph::new();
        let entry = fn_node(&mut graph, "entry", "entry.rs");
        let a = fn_node(&mut graph, "a", "cycle.rs");
        let b = fn_node(&mut graph, "b", "cycle.rs");
        let dst = fn_node(&mut graph, "dst", "dst.rs");
        link(&mut graph, entry, a);
        link(&mut graph, a, b);
        link(&mut graph, b, a);
        link(&mut graph, a, dst);

        let scores = compute_centrality(&graph, 30, 0.85);
        let sum: f64 = graph.node_indexes().map(|idx| scores.get_raw(idx)).sum();
        assert!(sum <= 1.0 + 1e-6, "non-sink cycle created mass, sum={sum}");
        assert!(scores.get_raw(dst) > scores.get_raw(a));
        assert!((scores.get_raw(a) - scores.get_raw(b)).abs() < 1e-9);

        // A longer cycle with the same entry and exit must not inflate a member
        // above the 2-cycle, and must not destroy mass relative to padding the
        // short graph out with dangling orphans (those hold mass that never moves).
        let (short_sum, short_member) = {
            let mut padded = ArborGraph::new();
            let entry = fn_node(&mut padded, "entry", "entry.rs");
            let a = fn_node(&mut padded, "a", "cycle.rs");
            let b = fn_node(&mut padded, "b", "cycle.rs");
            let dst = fn_node(&mut padded, "dst", "dst.rs");
            link(&mut padded, entry, a);
            link(&mut padded, a, b);
            link(&mut padded, b, a);
            link(&mut padded, a, dst);
            for i in 0..6 {
                fn_node(&mut padded, &format!("orphan{i}"), "orphan.rs");
            }
            let scores = compute_centrality(&padded, 30, 0.85);
            let sum: f64 = padded.node_indexes().map(|idx| scores.get_raw(idx)).sum();
            (sum, scores.get_raw(a))
        };
        let (long_sum, long_member) = {
            let mut padded = ArborGraph::new();
            let entry = fn_node(&mut padded, "entry", "entry.rs");
            let mut cycle = Vec::new();
            for i in 0..8 {
                cycle.push(fn_node(&mut padded, &format!("c{i}"), "cycle.rs"));
            }
            let dst = fn_node(&mut padded, "dst", "dst.rs");
            link(&mut padded, entry, cycle[0]);
            for i in 0..8 {
                link(&mut padded, cycle[i], cycle[(i + 1) % 8]);
            }
            link(&mut padded, cycle[0], dst);
            let scores = compute_centrality(&padded, 30, 0.85);
            let sum: f64 = padded.node_indexes().map(|idx| scores.get_raw(idx)).sum();
            (sum, scores.get_raw(cycle[0]))
        };
        assert!(
            long_member < short_member,
            "longer cycle inflated a member: {long_member} vs {short_member}"
        );
        assert!(
            long_sum + 1e-9 >= short_sum,
            "lengthening a non-sink cycle leaked mass: {long_sum} vs {short_sum}"
        );
    }

    #[test]
    fn test_file_weight_drops_transmitted_mass() {
        let build = |file: &str| {
            let mut graph = ArborGraph::new();
            let caller = fn_node(&mut graph, "caller", file);
            let target = fn_node(&mut graph, "target", "target.rs");
            link(&mut graph, caller, target);
            let scores = compute_centrality(&graph, 20, 0.85);
            let sum: f64 = graph.node_indexes().map(|idx| scores.get_raw(idx)).sum();
            (scores.get_raw(target), sum)
        };
        let (prod_target, prod_sum) = build("src/caller.rs");
        let (test_target, test_sum) = build("tests/caller_test.rs");
        assert!(prod_target > test_target);
        assert!(
            test_sum < prod_sum,
            "test weight 0.1 must drop mass, test_sum={test_sum} prod_sum={prod_sum}"
        );
    }

    #[test]
    fn iteration_budget_is_a_ceiling() {
        let mut graph = ArborGraph::new();
        let mut prev = fn_node(&mut graph, "n0", "chain.rs");
        let mut tail = prev;
        for i in 1..40 {
            let name = format!("n{i}");
            let cur = fn_node(&mut graph, &name, "chain.rs");
            link(&mut graph, prev, cur);
            prev = cur;
            tail = cur;
        }
        let at_20 = compute_centrality(&graph, 20, 0.85);
        let at_40 = compute_centrality(&graph, 40, 0.85);
        assert!(
            (at_40.get_raw(tail) - at_20.get_raw(tail)).abs() > 1e-8,
            "compute_centrality(..., 20, ...) must stop at 20 on a long chain"
        );
    }
}
