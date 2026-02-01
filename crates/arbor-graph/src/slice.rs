//! Dynamic context slicing for LLM prompts.
//!
//! This module provides token-bounded context extraction from the code graph.
//! Given a target node, it collects the minimal set of related nodes that fit
//! within a token budget.

use crate::graph::{ArborGraph, NodeId};
use crate::query::NodeInfo;
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::time::Instant;

/// Reason for stopping context collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TruncationReason {
    /// All reachable nodes within max_depth were included.
    Complete,
    /// Stopped because token budget was reached.
    TokenBudget,
    /// Stopped because max depth was reached.
    MaxDepth,
}

impl std::fmt::Display for TruncationReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TruncationReason::Complete => write!(f, "complete"),
            TruncationReason::TokenBudget => write!(f, "token_budget"),
            TruncationReason::MaxDepth => write!(f, "max_depth"),
        }
    }
}

/// A node included in the context slice.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextNode {
    /// Node information.
    pub node_info: NodeInfo,
    /// Estimated token count for this node's source.
    pub token_estimate: usize,
    /// Hop distance from the target node.
    pub depth: usize,
    /// Whether this node was pinned (always included).
    pub pinned: bool,
}

/// Result of a context slicing operation.
#[derive(Debug, Serialize, Deserialize)]
pub struct ContextSlice {
    /// The target node being queried.
    pub target: NodeInfo,
    /// Nodes included in the context, ordered by relevance.
    pub nodes: Vec<ContextNode>,
    /// Total estimated tokens in this slice.
    pub total_tokens: usize,
    /// Maximum tokens that were allowed.
    pub max_tokens: usize,
    /// Why slicing stopped.
    pub truncation_reason: TruncationReason,
    /// Query time in milliseconds.
    pub query_time_ms: u64,
}

impl ContextSlice {
    /// Returns a summary suitable for CLI output.
    pub fn summary(&self) -> String {
        format!(
            "Context: {} nodes, ~{} tokens ({})",
            self.nodes.len(),
            self.total_tokens,
            self.truncation_reason
        )
    }

    pub fn pinned_only(&self) -> Vec<&ContextNode> {
        self.nodes.iter().filter(|n| n.pinned).collect()
    }
}

use once_cell::sync::Lazy;
use tiktoken_rs::cl100k_base;

/// Global tokenizer instance (lazy-loaded once)
static TOKENIZER: Lazy<tiktoken_rs::CoreBPE> =
    Lazy::new(|| cl100k_base().expect("Failed to load cl100k_base tokenizer"));

/// Threshold for falling back to heuristic tokenizer (800 KB)
const LARGE_FILE_THRESHOLD: usize = 800 * 1024;

/// Estimates tokens for a code node using tiktoken cl100k_base.
///
/// Falls back to heuristic (4 chars/token) for large content to avoid
/// performance issues with massive JS bundles.
fn estimate_tokens(node: &NodeInfo) -> usize {
    let base = node.name.len() + node.qualified_name.len() + node.file.len();
    let signature_len = node.signature.as_ref().map(|s| s.len()).unwrap_or(0);
    let lines = (node.line_end.saturating_sub(node.line_start) + 1) as usize;
    let estimated_chars = base + signature_len + (lines * 40);

    // Performance guardrail: use heuristic for very large content
    if estimated_chars > LARGE_FILE_THRESHOLD {
        return (estimated_chars + 3) / 4; // Heuristic fallback
    }

    // Build text representation for accurate tokenization
    let text = format!(
        "{} {} {}{}",
        node.qualified_name,
        node.file,
        node.signature.as_deref().unwrap_or(""),
        " ".repeat(lines * 40) // Approximate code content
    );

    // Use tiktoken for accurate count
    TOKENIZER.encode_with_special_tokens(&text).len()
}

impl ArborGraph {
    /// Extracts a token-bounded context slice around a target node.
    ///
    /// Collects nodes in BFS order:
    /// 1. Target node itself
    /// 2. Direct upstream (callers) at depth 1
    /// 3. Direct downstream (callees) at depth 1
    /// 4. Continues outward until budget or max_depth reached
    ///
    /// Pinned nodes are always included regardless of budget.
    ///
    /// # Arguments
    /// * `target` - The node to center the slice around
    /// * `max_tokens` - Maximum token budget (0 = unlimited)
    /// * `max_depth` - Maximum hop distance (0 = unlimited, default: 2)
    /// * `pinned` - Nodes that must be included regardless of budget
    pub fn slice_context(
        &self,
        target: NodeId,
        max_tokens: usize,
        max_depth: usize,
        pinned: &[NodeId],
    ) -> ContextSlice {
        let start = Instant::now();

        let target_node = match self.get(target) {
            Some(node) => {
                let mut info = NodeInfo::from(node);
                info.centrality = self.centrality(target);
                info
            }
            None => {
                return ContextSlice {
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
                    nodes: Vec::new(),
                    total_tokens: 0,
                    max_tokens,
                    truncation_reason: TruncationReason::Complete,
                    query_time_ms: 0,
                };
            }
        };

        let effective_max = if max_depth == 0 {
            usize::MAX
        } else {
            max_depth
        };
        let effective_tokens = if max_tokens == 0 {
            usize::MAX
        } else {
            max_tokens
        };

        let pinned_set: HashSet<NodeId> = pinned.iter().copied().collect();
        let mut visited: HashSet<NodeId> = HashSet::new();
        let mut result: Vec<ContextNode> = Vec::new();
        let mut total_tokens = 0usize;
        let mut truncation_reason = TruncationReason::Complete;

        // BFS queue: (node_id, depth)
        let mut queue: VecDeque<(NodeId, usize)> = VecDeque::new();

        // Start with target
        queue.push_back((target, 0));

        while let Some((current, depth)) = queue.pop_front() {
            if visited.contains(&current) {
                continue;
            }

            if depth > effective_max {
                truncation_reason = TruncationReason::MaxDepth;
                continue;
            }

            visited.insert(current);

            let is_pinned = pinned_set.contains(&current);

            if let Some(node) = self.get(current) {
                let mut node_info = NodeInfo::from(node);
                node_info.centrality = self.centrality(current);

                let token_est = estimate_tokens(&node_info);

                // Check budget (pinned nodes bypass budget)
                let within_budget = is_pinned || total_tokens + token_est <= effective_tokens;

                if within_budget {
                    total_tokens += token_est;

                    result.push(ContextNode {
                        node_info,
                        token_estimate: token_est,
                        depth,
                        pinned: is_pinned,
                    });
                } else {
                    truncation_reason = TruncationReason::TokenBudget;
                    // Don't add to result, but STILL explore neighbors to find pinned nodes
                }
            }

            // Always add neighbors to queue (to find pinned nodes even when budget exceeded)
            if depth < effective_max {
                // Upstream (incoming)
                for edge_ref in self.graph.edges_directed(current, Direction::Incoming) {
                    let neighbor = edge_ref.source();
                    if !visited.contains(&neighbor) {
                        queue.push_back((neighbor, depth + 1));
                    }
                }

                // Downstream (outgoing)
                for edge_ref in self.graph.edges_directed(current, Direction::Outgoing) {
                    let neighbor = edge_ref.target();
                    if !visited.contains(&neighbor) {
                        queue.push_back((neighbor, depth + 1));
                    }
                }
            }
        }

        // Sort by: pinned first, then by depth, then by centrality (desc)
        result.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| a.depth.cmp(&b.depth))
                .then_with(|| {
                    b.node_info
                        .centrality
                        .partial_cmp(&a.node_info.centrality)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        let elapsed = start.elapsed().as_millis() as u64;

        ContextSlice {
            target: target_node,
            nodes: result,
            total_tokens,
            max_tokens,
            truncation_reason,
            query_time_ms: elapsed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::{Edge, EdgeKind};
    use arbor_core::{CodeNode, NodeKind};

    fn make_node(name: &str) -> CodeNode {
        CodeNode::new(name, name, NodeKind::Function, "test.rs")
    }

    #[test]
    fn test_empty_graph() {
        let graph = ArborGraph::new();
        let result = graph.slice_context(NodeId::new(0), 1000, 2, &[]);
        assert!(result.nodes.is_empty());
        assert_eq!(result.total_tokens, 0);
    }

    #[test]
    fn test_single_node() {
        let mut graph = ArborGraph::new();
        let id = graph.add_node(make_node("lonely"));

        let result = graph.slice_context(id, 1000, 2, &[]);
        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].node_info.name, "lonely");
        assert_eq!(result.truncation_reason, TruncationReason::Complete);
    }

    #[test]
    fn test_linear_chain_depth_limit() {
        // A → B → C → D
        let mut graph = ArborGraph::new();
        let a = graph.add_node(make_node("a"));
        let b = graph.add_node(make_node("b"));
        let c = graph.add_node(make_node("c"));
        let d = graph.add_node(make_node("d"));

        graph.add_edge(a, b, Edge::new(EdgeKind::Calls));
        graph.add_edge(b, c, Edge::new(EdgeKind::Calls));
        graph.add_edge(c, d, Edge::new(EdgeKind::Calls));

        // Slice from B with max_depth = 1
        let result = graph.slice_context(b, 10000, 1, &[]);

        // Should include B (depth 0), A (depth 1), C (depth 1)
        // D is depth 2, excluded
        let names: Vec<&str> = result
            .nodes
            .iter()
            .map(|n| n.node_info.name.as_str())
            .collect();
        assert!(names.contains(&"b"));
        assert!(names.contains(&"a"));
        assert!(names.contains(&"c"));
        assert!(!names.contains(&"d"));
    }

    #[test]
    fn test_token_budget() {
        let mut graph = ArborGraph::new();
        let a = graph.add_node(make_node("a"));
        let b = graph.add_node(make_node("b"));
        let c = graph.add_node(make_node("c"));

        graph.add_edge(a, b, Edge::new(EdgeKind::Calls));
        graph.add_edge(b, c, Edge::new(EdgeKind::Calls));

        // Very small budget - should truncate
        let result = graph.slice_context(a, 5, 10, &[]);

        // Should hit token budget
        assert!(result.nodes.len() < 3);
        assert_eq!(result.truncation_reason, TruncationReason::TokenBudget);
    }

    #[test]
    fn test_pinned_nodes_bypass_budget() {
        let mut graph = ArborGraph::new();
        let a = graph.add_node(make_node("a"));
        let b = graph.add_node(make_node("important_node"));
        let c = graph.add_node(make_node("c"));

        graph.add_edge(a, b, Edge::new(EdgeKind::Calls));
        graph.add_edge(b, c, Edge::new(EdgeKind::Calls));

        // Very small budget, but b is pinned
        let result = graph.slice_context(a, 5, 10, &[b]);

        // Pinned node should still be included
        let has_important = result
            .nodes
            .iter()
            .any(|n| n.node_info.name == "important_node");
        assert!(has_important);
    }

    #[test]
    fn test_complete_traversal() {
        let mut graph = ArborGraph::new();
        let a = graph.add_node(make_node("a"));
        let b = graph.add_node(make_node("b"));

        graph.add_edge(a, b, Edge::new(EdgeKind::Calls));

        // Large budget, should complete
        let result = graph.slice_context(a, 100000, 10, &[]);
        assert_eq!(result.truncation_reason, TruncationReason::Complete);
        assert_eq!(result.nodes.len(), 2);
    }
}
