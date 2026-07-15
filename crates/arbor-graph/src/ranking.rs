//! Centrality ranking for code nodes.
//!
//! We use a production-aware PageRank variant: callers from test files
//! contribute 10x less weight than production callers, so utility functions
//! called heavily by tests don't false-inflate centrality scores.

use crate::edge::EdgeKind;
use crate::graph::{ArborGraph, NodeId};
use petgraph::visit::{EdgeRef, IntoEdgeReferences};
use std::collections::HashMap;

/// Iteration stops early once no node's score moves more than this between
/// rounds. Tight enough that early exit is indistinguishable from running
/// the full iteration budget.
const CONVERGENCE_EPSILON: f64 = 1e-9;

/// Stores centrality scores after computation.
#[derive(Debug, Default)]
pub struct CentralityScores {
    scores: HashMap<NodeId, f64>,
}

impl CentralityScores {
    /// Gets the score for a node.
    pub fn get(&self, id: NodeId) -> f64 {
        self.scores.get(&id).copied().unwrap_or(0.0)
    }

    /// Converts to a HashMap for storage in the graph.
    pub fn into_map(self) -> HashMap<NodeId, f64> {
        self.scores
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
/// 4. Scores are normalized to [0.0, 1.0]
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
            // Stored scores are max-normalized, i.e. a scalar multiple c of the
            // iteration's fixed point. Left as-is they start far from it and the
            // warm start saves nothing. For v ≈ c·x*, summing the fixed-point
            // equation gives c = 1 − (f(v) − Σv) / (n·base) where
            // f(v) = n·base + damping·Σ gather(v) — so one pass over the edges
            // recovers c and v/c lands next to the fixed point.
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

    // Normalize to [0, 1] range
    let max_score = scores.iter().cloned().fold(0.0f64, f64::max);
    if max_score > 0.0 {
        for score in scores.iter_mut() {
            *score /= max_score;
        }
    }

    CentralityScores {
        scores: nodes.into_iter().zip(scores).collect(),
    }
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
        assert!(scores.scores.is_empty());
    }

    #[test]
    fn test_centrality_single_node() {
        let mut graph = ArborGraph::new();
        let node = CodeNode::new("foo", "foo", NodeKind::Function, "test.rs");
        graph.add_node(node);

        let scores = compute_centrality(&graph, 10, 0.85);
        assert_eq!(scores.scores.len(), 1);
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
}
