use arbor_core::CodeNode;
use arbor_graph::{AffectedNode, ArborGraph};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphStatsView {
    pub nodes: usize,
    pub edges: usize,
    pub files: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeSummary {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line_start: u32,
    pub line_end: u32,
    pub score: f64,
    pub signature: Option<String>,
}

impl NodeSummary {
    pub fn from_node(graph: &ArborGraph, node: &CodeNode) -> Self {
        let score = graph
            .get_index(&node.id)
            .map(|index| graph.centrality(index))
            .unwrap_or(0.0);

        Self {
            id: node.id.clone(),
            name: node.name.clone(),
            kind: node.kind.to_string(),
            file: node.file.clone(),
            line_start: node.line_start,
            line_end: node.line_end,
            score,
            signature: node.signature.clone(),
        }
    }

    pub fn from_affected(graph: &ArborGraph, affected: &AffectedNode) -> Self {
        Self {
            id: affected.node_info.id.clone(),
            name: affected.node_info.name.clone(),
            kind: affected.node_info.kind.clone(),
            file: affected.node_info.file.clone(),
            line_start: affected.node_info.line_start,
            line_end: affected.node_info.line_end,
            score: graph
                .get_index(&affected.node_info.id)
                .map(|index| graph.centrality(index))
                .unwrap_or(affected.node_info.centrality),
            signature: affected.node_info.signature.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexSummary {
    pub notes_path: String,
    pub db_path: String,
    pub files_indexed: usize,
    pub files_skipped: usize,
    pub files_removed: usize,
    pub nodes_written: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardSnapshot {
    pub notes_path: String,
    pub db_path: String,
    pub stats: GraphStatsView,
    pub top_nodes: Vec<NodeSummary>,
    pub sample_queries: Vec<String>,
    pub last_index_summary: Option<IndexSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<NodeSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImpactNode {
    #[serde(flatten)]
    pub node: NodeSummary,
    pub severity: String,
    pub hop_distance: usize,
    pub direction: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImpactReport {
    pub target: NodeSummary,
    pub summary: String,
    pub upstream: Vec<ImpactNode>,
    pub downstream: Vec<ImpactNode>,
    pub total_affected: usize,
    pub max_depth: usize,
    pub query_time_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathReport {
    pub from: NodeSummary,
    pub to: NodeSummary,
    pub nodes: Vec<NodeSummary>,
}
