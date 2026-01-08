//! Request handlers for protocol methods.
//!
//! Each handler implements one method from the Arbor Protocol.

use crate::protocol::{
    ContextParams, DiscoverParams, ImpactParams, NodeGetParams, Response, SearchParams,
};
use arbor_graph::{ArborGraph, NodeInfo};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::debug;

/// Shared state between handlers.
pub type SharedGraph = Arc<RwLock<ArborGraph>>;

/// Handles the graph.info method.
pub async fn handle_info(graph: SharedGraph, id: Option<Value>) -> Response {
    let g = graph.read().await;

    #[derive(Serialize)]
    struct InfoResult {
        #[serde(rename = "nodeCount")]
        node_count: usize,
        #[serde(rename = "edgeCount")]
        edge_count: usize,
        languages: Vec<&'static str>,
        version: &'static str,
    }

    Response::success(
        id,
        InfoResult {
            node_count: g.node_count(),
            edge_count: g.edge_count(),
            languages: vec!["typescript", "rust", "python"],
            version: env!("CARGO_PKG_VERSION"),
        },
    )
}

/// Handles the discover method.
pub async fn handle_discover(
    graph: SharedGraph,
    id: Option<Value>,
    params: DiscoverParams,
) -> Response {
    let start = Instant::now();
    let g = graph.read().await;

    debug!("Discover query: {}", params.query);

    // Search for nodes matching the query
    let mut matches: Vec<_> = g
        .search(&params.query)
        .into_iter()
        .map(|node| {
            let centrality = g.centrality(g.get_index(&node.id).unwrap_or_default());
            let mut info = NodeInfo::from(node);
            info.centrality = centrality;
            info
        })
        .collect();

    // Sort by centrality (most important first)
    matches.sort_by(|a, b| {
        b.centrality
            .partial_cmp(&a.centrality)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Limit results
    matches.truncate(params.limit);

    #[derive(Serialize)]
    struct DiscoverResult {
        nodes: Vec<NodeInfo>,
        #[serde(rename = "queryTime")]
        query_time: u64,
    }

    Response::success(
        id,
        DiscoverResult {
            nodes: matches,
            query_time: start.elapsed().as_millis() as u64,
        },
    )
}

/// Handles the impact method.
pub async fn handle_impact(
    graph: SharedGraph,
    id: Option<Value>,
    params: ImpactParams,
) -> Response {
    let start = Instant::now();
    let g = graph.read().await;

    debug!("Impact analysis for: {}", params.node);

    // Find the target node
    let target_idx = match g.get_index(&params.node) {
        Some(idx) => idx,
        None => {
            return Response::error(id, -32001, format!("Node not found: {}", params.node));
        }
    };

    let target = g.get(target_idx).map(NodeInfo::from);

    // Get dependents
    let dependents = g.get_dependents(target_idx, params.depth);
    let total = dependents.len();

    let dependent_infos: Vec<_> = dependents
        .into_iter()
        .filter_map(|(idx, depth)| {
            let node = g.get(idx)?;
            Some(serde_json::json!({
                "node": NodeInfo::from(node),
                "relationship": "calls",
                "depth": depth
            }))
        })
        .collect();

    Response::success(
        id,
        serde_json::json!({
            "target": target,
            "dependents": dependent_infos,
            "totalAffected": total,
            "queryTime": start.elapsed().as_millis()
        }),
    )
}

/// Handles the context method.
pub async fn handle_context(
    graph: SharedGraph,
    id: Option<Value>,
    params: ContextParams,
) -> Response {
    let start = Instant::now();
    let g = graph.read().await;

    debug!("Context request for task: {}", params.task);

    // Search for relevant nodes
    let mut matches: Vec<_> = g
        .search(&params.task)
        .into_iter()
        .map(|node| {
            let centrality = g.centrality(g.get_index(&node.id).unwrap_or_default());
            let mut info = NodeInfo::from(node);
            info.centrality = centrality;
            info
        })
        .collect();

    // Sort by centrality
    matches.sort_by(|a, b| {
        b.centrality
            .partial_cmp(&a.centrality)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Estimate tokens and truncate
    // (Rough estimate: 4 characters per token)
    let mut total_tokens = 0usize;
    let mut selected = Vec::new();

    for node in matches {
        let estimated_tokens = (node.line_end - node.line_start) as usize * 40 / 4;
        if total_tokens + estimated_tokens > params.max_tokens {
            break;
        }
        total_tokens += estimated_tokens;
        selected.push(node);
    }

    Response::success(
        id,
        serde_json::json!({
            "nodes": selected,
            "totalTokens": total_tokens,
            "queryTime": start.elapsed().as_millis()
        }),
    )
}

/// Handles the search method.
pub async fn handle_search(
    graph: SharedGraph,
    id: Option<Value>,
    params: SearchParams,
) -> Response {
    let start = Instant::now();
    let g = graph.read().await;

    debug!("Search: {}", params.query);

    let mut matches: Vec<_> = g
        .search(&params.query)
        .into_iter()
        .filter(|node| {
            // Filter by kind if specified
            if let Some(ref kind) = params.kind {
                node.kind.to_string() == *kind
            } else {
                true
            }
        })
        .map(NodeInfo::from)
        .collect();

    let total = matches.len();
    matches.truncate(params.limit);

    Response::success(
        id,
        serde_json::json!({
            "nodes": matches,
            "total": total,
            "queryTime": start.elapsed().as_millis()
        }),
    )
}

/// Handles the node.get method.
pub async fn handle_node_get(
    graph: SharedGraph,
    id: Option<Value>,
    params: NodeGetParams,
) -> Response {
    let g = graph.read().await;

    match g.get_by_id(&params.id) {
        Some(node) => {
            let idx = match g.get_index(&node.id) {
                Some(i) => i,
                None => {
                    return Response::error(
                        id,
                        -32603,
                        format!("Internal error: Node index not found for {}", node.id),
                    );
                }
            };
            let callers: Vec<_> = g.get_callers(idx).iter().map(|n| &n.id).collect();
            let callees: Vec<_> = g.get_callees(idx).iter().map(|n| &n.id).collect();

            Response::success(
                id,
                serde_json::json!({
                    "id": node.id,
                    "name": node.name,
                    "qualifiedName": node.qualified_name,
                    "kind": node.kind.to_string(),
                    "file": node.file,
                    "lineStart": node.line_start,
                    "lineEnd": node.line_end,
                    "signature": node.signature,
                    "edges": {
                        "calledBy": callers,
                        "calls": callees
                    }
                }),
            )
        }
        None => Response::error(id, -32001, format!("Node not found: {}", params.id)),
    }
}
