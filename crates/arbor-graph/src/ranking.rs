//! Centrality ranking for code nodes.
//!
//! We use a production-aware PageRank variant: callers from test files
//! contribute 10x less weight than production callers, so utility functions
//! called heavily by tests don't false-inflate centrality scores.

use crate::edge::EdgeKind;
use crate::graph::{ArborGraph, NodeId};
use arbor_core::NodeKind;
use petgraph::visit::{EdgeRef, IntoEdgeReferences};
use std::collections::HashMap;
use std::path::Path;

/// Iteration stops early once no node's score moves more than this between
/// rounds. Tight enough that early exit is indistinguishable from running
/// the full iteration budget.
const CONVERGENCE_EPSILON: f64 = 1e-9;

/// Centrality scores in two forms.
///
/// # Why two
///
/// Raw PageRank mass sums to 1.0 across the graph, so an individual value
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

    /// Raw PageRank mass. Sums to ~1.0 across the graph.
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
    /// so tied nodes share a rank and the ordering is total and deterministic.
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

/// Computes production-aware centrality scores for all nodes in the graph.
///
/// Uses a modified PageRank where:
/// 1. Nodes initialize with equal score
/// 2. Each iteration distributes scores along edges
/// 3. Callers from test/spec/fixture files contribute 10x less weight
///    — prevents test utilities from appearing more central than production code
/// 4. Raw scores are converted to percentile ranks for cross-repo comparability
///    (see [`CentralityScores`])
///
/// # Arguments
///
/// * `graph` - The graph to analyze
/// * `iterations` - Number of iterations (10-20 is usually enough)
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

    // One pass over the edges builds the call adjacency: out-degrees for the
    // score split, and per-node caller lists for the gather.
    let mut out_degree: Vec<usize> = vec![0; n];
    let mut in_edges: Vec<Vec<u32>> = vec![Vec::new(); n];
    for edge in graph.graph.edge_references() {
        if edge.weight().kind != EdgeKind::Calls {
            continue;
        }
        let (Some(&source), Some(&target)) = (pos.get(&edge.source()), pos.get(&edge.target()))
        else {
            continue;
        };
        out_degree[source] += 1;
        in_edges[target].push(source as u32);
    }
    for degree in out_degree.iter_mut() {
        *degree = (*degree).max(1);
    }

    let initial_score = 1.0 / n as f64;
    let base = (1.0 - damping) / n as f64;
    let gather = |scores: &[f64], target: usize| -> f64 {
        in_edges[target]
            .iter()
            .map(|&source| {
                let source = source as usize;
                weights[source] * scores[source] / out_degree[source] as f64
            })
            .sum()
    };

    let mut scores: Vec<f64> = match previous {
        Some(prev) if !prev.is_empty() => {
            let mut warm: Vec<f64> = nodes
                .iter()
                .map(|id| prev.get(id).copied().unwrap_or(initial_score))
                .collect();
            // Stored scores may be any scalar multiple c of the iteration's
            // fixed point. For raw scores (what `centrality_map` now returns)
            // c ≈ 1 and this is a no-op; the rescale is kept so a caller that
            // hands us normalized scores still converges. For v ≈ c·x*, summing
            // the fixed-point equation gives c = 1 − (f(v) − Σv) / (n·base)
            // where f(v) = n·base + damping·Σ gather(v) — so one pass over the
            // edges recovers c and v/c lands next to the fixed point.
            let sum_v: f64 = warm.iter().sum();
            let f_v: f64 =
                n as f64 * base + damping * (0..n).map(|t| gather(&warm, t)).sum::<f64>();
            let c = 1.0 - (f_v - sum_v) / (n as f64 * base);
            if c.is_finite() && c > f64::EPSILON {
                for score in warm.iter_mut() {
                    *score /= c;
                }
            }
            warm
        }
        _ => vec![initial_score; n],
    };

    let mut next: Vec<f64> = vec![0.0; n];
    for _ in 0..iterations {
        let mut max_delta = 0.0f64;
        for target in 0..n {
            let score = base + damping * gather(&scores, target);
            max_delta = max_delta.max((score - scores[target]).abs());
            next[target] = score;
        }
        std::mem::swap(&mut scores, &mut next);
        if max_delta < CONVERGENCE_EPSILON {
            break;
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
}
