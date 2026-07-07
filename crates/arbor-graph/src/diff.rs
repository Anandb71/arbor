//! Git-diff blast radius computation shared by CLI and MCP.

use crate::{ArborGraph, NodeId};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

/// Summary of blast radius for changed files (matches CLI `arbor diff` output).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlastRadiusSummary {
    pub changed_files: Vec<String>,
    pub changed_symbols: usize,
    pub direct_callers: usize,
    pub indirect_callers: usize,
    pub entrypoints_affected: usize,
    pub files_likely_updates: usize,
    pub blast_radius_nodes: usize,
    pub mermaid_diagram: Option<String>,
    pub risk_level: String,
}

fn normalize_slashes(input: &str) -> String {
    input.replace('\\', "/")
}

/// Whether a graph node's file path matches a git-changed path.
pub fn node_matches_changed_file(node_file: &str, changed_file: &str, project_root: &Path) -> bool {
    let node_norm = normalize_slashes(node_file);
    let changed_norm = normalize_slashes(changed_file);

    if node_norm.ends_with(&changed_norm) {
        return true;
    }

    let abs = project_root.join(&changed_norm);
    let abs_norm = normalize_slashes(&abs.to_string_lossy());
    node_norm == abs_norm
}

/// Collect node IDs whose files appear in the changed-files list.
pub fn changed_node_ids(
    graph: &ArborGraph,
    changed_files: &[String],
    project_root: &Path,
) -> Vec<NodeId> {
    graph
        .node_indexes()
        .filter(|idx| {
            graph.get(*idx).is_some_and(|node| {
                changed_files
                    .iter()
                    .any(|f| node_matches_changed_file(&node.file, f, project_root))
            })
        })
        .collect()
}

fn risk_level_for(blast_radius_nodes: usize) -> String {
    if blast_radius_nodes > 50 {
        "critical".to_string()
    } else if blast_radius_nodes > 25 {
        "high".to_string()
    } else if blast_radius_nodes > 10 {
        "medium".to_string()
    } else {
        "low".to_string()
    }
}

/// Compute blast radius summary from an indexed graph and changed file list.
pub fn compute_blast_radius(
    graph: &ArborGraph,
    changed_files: Vec<String>,
    changed_node_ids: Vec<NodeId>,
    max_depth: usize,
    project_root: &Path,
) -> BlastRadiusSummary {
    let mut direct_callers = HashSet::new();
    let mut indirect_callers = HashSet::new();
    let mut affected_nodes = HashSet::new();
    let mut affected_files = HashSet::new();

    for node_id in changed_node_ids.iter().copied() {
        let analysis = graph.analyze_impact(node_id, max_depth);

        for up in &analysis.upstream {
            affected_nodes.insert(up.node_info.id.clone());
            affected_files.insert(up.node_info.file.clone());
            if up.hop_distance <= 1 {
                direct_callers.insert(up.node_info.id.clone());
            } else {
                indirect_callers.insert(up.node_info.id.clone());
            }
        }

        for down in &analysis.downstream {
            affected_nodes.insert(down.node_info.id.clone());
            affected_files.insert(down.node_info.file.clone());
        }
    }

    let entrypoints_affected = affected_nodes
        .iter()
        .filter_map(|id| graph.get_index(id))
        .filter(|idx| graph.analyze_impact(*idx, 1).upstream.is_empty())
        .count();

    let changed_norm: Vec<String> = changed_files.iter().map(|f| normalize_slashes(f)).collect();
    let files_likely_updates = affected_files
        .iter()
        .filter(|f| {
            let f_norm = normalize_slashes(f);
            !changed_norm.iter().any(|c| {
                f_norm.ends_with(c)
                    || f_norm == normalize_slashes(&project_root.join(c).to_string_lossy())
            })
        })
        .count();

    let mut mermaid_lines = Vec::new();
    mermaid_lines.push("graph TD".to_string());
    mermaid_lines.push(
        "  classDef changed fill:#ef4444,stroke:#333,stroke-width:2px,color:#fff;".to_string(),
    );
    mermaid_lines.push(
        "  classDef caller fill:#f59e0b,stroke:#333,stroke-width:1px,color:#fff;".to_string(),
    );

    let mut added_edges = HashSet::new();
    let mut changed_node_names = HashSet::new();
    let mut direct_caller_names = HashSet::new();

    for node_id in changed_node_ids.iter().copied().take(5) {
        if let Some(node) = graph.get(node_id) {
            let target_name = node.name.replace([':', '<', '>', '(', ')', '[', ']'], "_");
            changed_node_names.insert(target_name.clone());

            let analysis = graph.analyze_impact(node_id, max_depth);

            let mut caller_count = 0;
            for up in &analysis.upstream {
                if up.hop_distance == 1 {
                    let caller_name = up
                        .node_info
                        .name
                        .replace([':', '<', '>', '(', ')', '[', ']'], "_");
                    direct_caller_names.insert(caller_name.clone());

                    let edge = format!(
                        "  {}[{}] --> {}[{}]",
                        caller_name, up.node_info.name, target_name, node.name
                    );
                    if added_edges.insert(edge.clone()) {
                        mermaid_lines.push(edge);
                        caller_count += 1;
                        if caller_count >= 3 {
                            break;
                        }
                    }
                }
            }
        }
    }

    for name in &changed_node_names {
        mermaid_lines.push(format!("  class {} changed;", name));
    }
    for name in &direct_caller_names {
        if !changed_node_names.contains(name) {
            mermaid_lines.push(format!("  class {} caller;", name));
        }
    }

    let mermaid_diagram = if mermaid_lines.len() > 3 {
        Some(mermaid_lines.join("\n"))
    } else {
        None
    };

    let blast_radius_nodes = affected_nodes.len();

    BlastRadiusSummary {
        changed_files,
        changed_symbols: changed_node_ids.len(),
        direct_callers: direct_callers.len(),
        indirect_callers: indirect_callers.len(),
        entrypoints_affected,
        files_likely_updates,
        blast_radius_nodes,
        mermaid_diagram,
        risk_level: risk_level_for(blast_radius_nodes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArborGraph;
    use arbor_core::{CodeNode, NodeKind};

    #[test]
    fn compute_blast_radius_empty_changes() {
        let graph = ArborGraph::new();
        let summary = compute_blast_radius(&graph, vec![], vec![], 5, Path::new("."));
        assert_eq!(summary.blast_radius_nodes, 0);
        assert_eq!(summary.risk_level, "low");
    }

    #[test]
    fn changed_node_ids_finds_nodes_in_file() {
        let mut graph = ArborGraph::new();
        let node = CodeNode::new("foo", "foo", NodeKind::Function, "src/lib.rs");
        let id = graph.add_node(node);

        let ids = changed_node_ids(&graph, &["src/lib.rs".to_string()], Path::new("."));
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], id);
    }
}
