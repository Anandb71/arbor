use std::path::Path;
use anyhow::{Context, Result};
use serde::Serialize;
use arbor_graph::{ArborGraph, NodeInfo, NodeId}; // access re-exports from lib.rs

#[derive(Debug, Serialize, Clone)]
pub struct AuditResult {
    /// The sensitive sink being audited (e.g., "db_query", "exec")
    pub sink: NodeInfo,
    /// Paths from public entry points to this sink
    pub entry_points: Vec<AuditPath>,
    /// Total number of reachable paths found
    pub path_count: usize,
    /// Confidence score of this audit (0.0 - 1.0)
    pub confidence: f32,
}

#[derive(Debug, Serialize, Clone)]
pub struct AuditPath {
    /// The public entry point (source)
    pub source: NodeInfo,
    /// The sequence of nodes from source to sink
    pub trace: Vec<NodeInfo>,
    /// Any uncertain edges encountered along this path
    pub uncertainty: Vec<String>,
}

pub struct AuditConfig {
    pub max_depth: usize,
    pub ignore_tests: bool,
}

impl AuditResult {
    pub fn new(sink: NodeInfo) -> Self {
        Self {
            sink,
            entry_points: Vec::new(),
            path_count: 0,
            confidence: 1.0,
        }
    }
}

pub fn run_audit(graph: &ArborGraph, sink_name: &str, config: &AuditConfig) -> Result<AuditResult> {
    // 1. Find the sink node
    let nodes = graph.find_by_name(sink_name);
    if nodes.is_empty() {
        return Err(anyhow::anyhow!("Sink symbol '{}' not found in graph", sink_name));
    }
    
    // Warn if multiple? For now just take the first one.
    // In future we could support interactive selection or "audit all".
    let sink_node = nodes[0];
    let sink_id = graph.get_index(&sink_node.id).unwrap();
    
    let mut result = AuditResult::new(NodeInfo::from(sink_node));

    // 2. Perform BFS/DFS upstream to find entry points
    let mut paths = Vec::new();
    find_paths_recursive(graph, sink_id, &mut Vec::new(), &mut paths, config.max_depth);

    // 3. Convert raw paths to AuditPath
    for path_ids in paths {
        if let Some(source_id) = path_ids.last() {
             let source_node = graph.get(*source_id).unwrap();
             let trace: Vec<NodeInfo> = path_ids.iter().rev()
                .map(|id| NodeInfo::from(graph.get(*id).unwrap()))
                .collect();
             
             let uncertainty = Vec::new(); 

             result.entry_points.push(AuditPath {
                 source: NodeInfo::from(source_node),
                 trace,
                 uncertainty,
             });
        }
    }

    result.path_count = result.entry_points.len();
    result.entry_points.sort_by_key(|p| p.trace.len());

    Ok(result)
}

fn find_paths_recursive(
    graph: &ArborGraph,
    current_node: NodeId,
    current_path: &mut Vec<NodeId>,
    all_paths: &mut Vec<Vec<NodeId>>,
    depth_remaining: usize,
) {
    current_path.push(current_node);

    if depth_remaining == 0 {
        current_path.pop();
        return;
    }

    // Find callers (incoming edges) via public API
    let callers_nodes = graph.get_callers(current_node);

    if callers_nodes.is_empty() {
        // This is a root / entry point
        all_paths.push(current_path.clone());
    } else {
        for caller_node in callers_nodes {
            // Retrieve ID for the caller node
            if let Some(caller_id) = graph.get_index(&caller_node.id) {
                 // Avoid cycles
                if !current_path.contains(&caller_id) {
                    find_paths_recursive(graph, caller_id, current_path, all_paths, depth_remaining - 1);
                }
            }
        }
    }

    current_path.pop();
}
