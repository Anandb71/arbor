//! Security audit module for Arbor.
//!
//! Traces paths from public entry points to sensitive sinks,
//! enabling blast-radius analysis for CVEs and security reviews.

use anyhow::Result;
use arbor_graph::{ArborGraph, NodeId, NodeInfo};
use serde::Serialize;

/// Severity level of an audit finding based on path characteristics.
#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Direct call (depth 1-2) — immediate exposure
    Critical,
    /// Short path (depth 3-4) — high risk
    High,
    /// Medium path (depth 5-6) — moderate risk
    Medium,
    /// Long path (depth 7+) — lower but nonzero risk
    Low,
}

impl Severity {
    pub fn from_depth(depth: usize) -> Self {
        match depth {
            0..=2 => Severity::Critical,
            3..=4 => Severity::High,
            5..=6 => Severity::Medium,
            _ => Severity::Low,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Severity::Critical => "CRITICAL",
            Severity::High => "HIGH",
            Severity::Medium => "MEDIUM",
            Severity::Low => "LOW",
        }
    }

    pub fn emoji(&self) -> &'static str {
        match self {
            Severity::Critical => "🔴",
            Severity::High => "🟠",
            Severity::Medium => "🟡",
            Severity::Low => "🟢",
        }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Complete result of a security audit against a single sink.
#[derive(Debug, Serialize, Clone)]
pub struct AuditResult {
    /// The sensitive sink being audited (e.g., "db_query", "exec")
    pub sink: NodeInfo,
    /// Paths from public entry points to this sink
    pub paths: Vec<AuditPath>,
    /// Total number of reachable paths found
    pub path_count: usize,
    /// Confidence score of this audit (0.0 - 1.0)
    pub confidence: f32,
    /// Summary statistics
    pub summary: AuditSummary,
}

/// Statistics summary for the audit.
#[derive(Debug, Serialize, Clone, Default)]
pub struct AuditSummary {
    pub critical_count: usize,
    pub high_count: usize,
    pub medium_count: usize,
    pub low_count: usize,
    pub unique_entry_points: usize,
    pub unique_files: usize,
}

/// A single path from entry point to sink.
#[derive(Debug, Serialize, Clone)]
pub struct AuditPath {
    /// The public entry point (source)
    pub source: NodeInfo,
    /// The sequence of nodes from source to sink
    pub trace: Vec<NodeInfo>,
    /// Severity based on path length
    pub severity: Severity,
    /// Any uncertain edges encountered along this path
    pub uncertainty: Vec<String>,
}

/// Configuration for the audit engine.
pub struct AuditConfig {
    pub max_depth: usize,
    pub ignore_tests: bool,
}

impl AuditResult {
    fn new(sink: NodeInfo) -> Self {
        Self {
            sink,
            paths: Vec::new(),
            path_count: 0,
            confidence: 1.0,
            summary: AuditSummary::default(),
        }
    }

    /// Build the summary statistics from collected paths.
    fn compute_summary(&mut self) {
        let mut entry_names = std::collections::HashSet::new();
        let mut files = std::collections::HashSet::new();

        for path in &self.paths {
            entry_names.insert(path.source.name.clone());
            for node in &path.trace {
                files.insert(node.file.clone());
            }
            match path.severity {
                Severity::Critical => self.summary.critical_count += 1,
                Severity::High => self.summary.high_count += 1,
                Severity::Medium => self.summary.medium_count += 1,
                Severity::Low => self.summary.low_count += 1,
            }
        }

        self.summary.unique_entry_points = entry_names.len();
        self.summary.unique_files = files.len();
    }
}

/// Returns true if the file path looks like a test file.
fn is_test_file(file: &str) -> bool {
    let lower = file.to_lowercase();
    lower.contains("test")
        || lower.contains("spec")
        || lower.contains("__tests__")
        || lower.ends_with("_test.rs")
        || lower.ends_with("_test.go")
        || lower.ends_with(".test.ts")
        || lower.ends_with(".test.js")
        || lower.ends_with(".spec.ts")
        || lower.ends_with(".spec.js")
}

/// Run a security audit: find all paths from entry points to the given sink.
pub fn run_audit(graph: &ArborGraph, sink_name: &str, config: &AuditConfig) -> Result<AuditResult> {
    // 1. Find the sink node
    let nodes = graph.find_by_name(sink_name);
    if nodes.is_empty() {
        return Err(anyhow::anyhow!(
            "Sink symbol '{}' not found in graph",
            sink_name
        ));
    }

    let sink_node = nodes[0];
    let sink_id = graph
        .get_index(&sink_node.id)
        .ok_or_else(|| anyhow::anyhow!("Sink node has no graph index"))?;

    let mut result = AuditResult::new(NodeInfo::from(sink_node));

    // 2. Reverse-traverse the call graph to find all paths to entry points
    let mut raw_paths = Vec::new();
    find_paths_to_roots(
        graph,
        sink_id,
        &mut Vec::new(),
        &mut raw_paths,
        config.max_depth,
        config.ignore_tests,
    );

    // 3. Convert raw NodeId paths into structured AuditPaths
    for path_ids in raw_paths {
        if let Some(source_id) = path_ids.last() {
            if let Some(source_node) = graph.get(*source_id) {
                let trace: Vec<NodeInfo> = path_ids
                    .iter()
                    .rev()
                    .filter_map(|id| graph.get(*id).map(NodeInfo::from))
                    .collect();

                let severity = Severity::from_depth(trace.len());

                result.paths.push(AuditPath {
                    source: NodeInfo::from(source_node),
                    trace,
                    severity,
                    uncertainty: Vec::new(),
                });
            }
        }
    }

    // 4. Sort by severity (critical first), then by path length
    result.paths.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.trace.len().cmp(&b.trace.len()))
    });

    result.path_count = result.paths.len();
    result.compute_summary();

    Ok(result)
}

/// DFS traversal upstream (callers) to find all paths from entry points to the current node.
fn find_paths_to_roots(
    graph: &ArborGraph,
    current: NodeId,
    path: &mut Vec<NodeId>,
    results: &mut Vec<Vec<NodeId>>,
    depth_remaining: usize,
    ignore_tests: bool,
) {
    path.push(current);

    if depth_remaining == 0 {
        path.pop();
        return;
    }

    let callers = graph.get_callers(current);

    if callers.is_empty() {
        // Reached a root — this is an entry point
        results.push(path.clone());
    } else {
        for caller in callers {
            // Skip test files if configured
            if ignore_tests && is_test_file(&caller.file) {
                continue;
            }

            if let Some(caller_id) = graph.get_index(&caller.id) {
                // Skip cycles
                if !path.contains(&caller_id) {
                    find_paths_to_roots(
                        graph,
                        caller_id,
                        path,
                        results,
                        depth_remaining - 1,
                        ignore_tests,
                    );
                }
            }
        }
    }

    path.pop();
}
