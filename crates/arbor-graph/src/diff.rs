//! Git-diff blast radius computation shared by CLI and MCP.

use crate::{ArborGraph, NodeId};
use arbor_core::NodeKind;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
///
/// This is *file* granularity: every symbol in a touched file is treated as
/// changed. For a one-line edit in a 60-symbol file that overstates the blast
/// radius by roughly 60x. Prefer [`changed_node_ids_for_ranges`] whenever the
/// caller has the diff hunks — which any PR-driven integration does.
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

/// A contiguous run of changed lines within one file.
///
/// Lines are 1-indexed and both bounds are inclusive, matching how editors and
/// `CodeNode::line_start`/`line_end` count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedRange {
    pub file: String,
    pub start_line: u32,
    pub end_line: u32,
}

impl ChangedRange {
    pub fn new(file: impl Into<String>, start_line: u32, end_line: u32) -> Self {
        Self {
            file: file.into(),
            start_line,
            end_line,
        }
    }

    /// Whether this range overlaps an inclusive `[start, end]` span.
    fn overlaps(&self, start: u32, end: u32) -> bool {
        self.start_line <= end && start <= self.end_line
    }
}

/// Symbols touched by a set of changed line ranges.
#[derive(Debug, Clone, Default)]
pub struct ChangedSymbols {
    /// Nodes overlapping at least one changed range, in graph order.
    pub node_ids: Vec<NodeId>,

    /// Files that changed but where no symbol overlapped the changed lines.
    ///
    /// Usually an import block, a top-level constant, a comment, or a
    /// whitespace-only edit. Surfacing these lets a report say "this file
    /// changed outside any tracked symbol" instead of silently reporting
    /// zero impact — an honest unknown rather than an invisible one.
    pub files_without_symbol_hits: Vec<String>,
}

/// Collect node IDs whose line spans overlap the given changed ranges.
///
/// A symbol counts as changed when any changed line falls inside its span.
/// Symbols elsewhere in the same file are left out, which is the whole point:
/// editing one function should not implicate its 59 neighbours.
///
/// Nodes whose line span is unset (`line_end == 0`, which some fallback
/// parsers produce) cannot be positioned, so they are included whenever their
/// file is touched. That is the conservative choice — better a slightly wide
/// radius than a silently missing symbol.
///
/// A container (a module, class, trait, struct or enum) counts only when a
/// changed line inside it falls outside all of its members: editing a method
/// changes the method, not `mod mac` or `class Session` around it. A field, a
/// `use` or the header itself still counts. Functions are not containers here:
/// changing a closure inside one changes what the function does.
pub fn changed_node_ids_for_ranges(
    graph: &ArborGraph,
    ranges: &[ChangedRange],
    project_root: &Path,
) -> ChangedSymbols {
    let mut node_ids = Vec::new();
    let mut files_hit: HashSet<String> = HashSet::new();

    for idx in graph.node_indexes() {
        let Some(node) = graph.get(idx) else {
            continue;
        };

        for range in ranges {
            if !node_matches_changed_file(&node.file, &range.file, project_root) {
                continue;
            }

            // Position-unknown nodes fall back to file granularity.
            let positioned = node.line_end > 0;
            if !positioned || range.overlaps(node.line_start, node.line_end) {
                node_ids.push(idx);
                files_hit.insert(range.file.clone());
                break;
            }
        }
    }

    let node_ids = drop_explained_containers(graph, node_ids, ranges, project_root);

    let mut files_without_symbol_hits: Vec<String> = ranges
        .iter()
        .map(|r| r.file.clone())
        .filter(|f| !files_hit.contains(f))
        .collect();
    files_without_symbol_hits.sort();
    files_without_symbol_hits.dedup();

    ChangedSymbols {
        node_ids,
        files_without_symbol_hits,
    }
}

fn is_container(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Module
            | NodeKind::Class
            | NodeKind::Interface
            | NodeKind::Struct
            | NodeKind::Enum
    )
}

/// Removes containers whose changed lines all fall inside their members.
fn drop_explained_containers(
    graph: &ArborGraph,
    node_ids: Vec<NodeId>,
    ranges: &[ChangedRange],
    project_root: &Path,
) -> Vec<NodeId> {
    let containers: Vec<NodeId> = node_ids
        .iter()
        .copied()
        .filter(|id| {
            graph
                .get(*id)
                .is_some_and(|node| is_container(node.kind) && node.line_end > 0)
        })
        .collect();
    if containers.is_empty() {
        return node_ids;
    }

    // Positioned spans of every symbol in the files that hold a container.
    let files: HashSet<&str> = containers
        .iter()
        .filter_map(|id| graph.get(*id).map(|node| node.file.as_str()))
        .collect();
    let mut spans: HashMap<&str, Vec<(u32, u32)>> = HashMap::new();
    for idx in graph.node_indexes() {
        if let Some(node) = graph.get(idx) {
            if node.line_end > 0 && files.contains(node.file.as_str()) {
                spans
                    .entry(node.file.as_str())
                    .or_default()
                    .push((node.line_start, node.line_end));
            }
        }
    }

    let explained: HashSet<NodeId> = containers
        .into_iter()
        .filter(|id| {
            let Some(container) = graph.get(*id) else {
                return false;
            };
            let (start, end) = (container.line_start, container.line_end);
            let mut members: Vec<(u32, u32)> = spans
                .get(container.file.as_str())
                .map(|all| {
                    all.iter()
                        .copied()
                        .filter(|&(s, e)| s >= start && e <= end && (s, e) != (start, end))
                        .collect()
                })
                .unwrap_or_default();
            members.sort_unstable();
            ranges
                .iter()
                .filter(|range| {
                    node_matches_changed_file(&container.file, &range.file, project_root)
                })
                .filter(|range| range.overlaps(start, end))
                .all(|range| {
                    covered(
                        range.start_line.max(start),
                        range.end_line.min(end),
                        &members,
                    )
                })
        })
        .collect();

    node_ids
        .into_iter()
        .filter(|id| !explained.contains(id))
        .collect()
}

/// Whether every line in `[start, end]` lies inside one of the sorted `spans`.
fn covered(start: u32, end: u32, spans: &[(u32, u32)]) -> bool {
    let mut next = start;
    for &(s, e) in spans {
        if s > next {
            break;
        }
        if e >= next {
            next = e.saturating_add(1);
            if next > end {
                return true;
            }
        }
    }
    next > end
}

/// Extract changed line ranges from a unified diff patch for one file.
///
/// Reads the `+` side of each `@@ -a,b +c,d @@` header, so the ranges refer to
/// line numbers in the *new* file — the same numbering the indexed graph uses.
///
/// The full hunk span is returned, including its context lines. Context is
/// deliberately kept: a hunk that only deletes lines has no added lines to
/// point at, but the deletion still changes the enclosing symbol, and the
/// surrounding context is what locates it. Over-including by the three context
/// lines either side is immaterial next to the alternative of taking the whole
/// file.
///
/// Malformed headers are skipped rather than failing the parse — a diff that
/// partially parses still yields a far better radius than no ranges at all.
pub fn parse_unified_diff_ranges(patch: &str, file: &str) -> Vec<ChangedRange> {
    let mut ranges = Vec::new();

    for line in patch.lines() {
        if !line.starts_with("@@") {
            continue;
        }

        // `@@ -12,7 +12,9 @@ fn context()` → we want the `+12,9`.
        let Some(rest) = line.strip_prefix("@@") else {
            continue;
        };
        let Some(plus) = rest.split('+').nth(1) else {
            continue;
        };
        let spec = plus.split_whitespace().next().unwrap_or("");
        let mut parts = spec.split(',');

        let Some(Ok(start)) = parts.next().map(str::parse::<u32>) else {
            continue;
        };
        // An omitted count means a single line.
        let count = match parts.next() {
            Some(c) => match c.parse::<u32>() {
                Ok(n) => n,
                Err(_) => continue,
            },
            None => 1,
        };

        if count == 0 {
            // Pure deletion: the new file has no lines here. Anchor on the
            // insertion point so the enclosing symbol is still found.
            ranges.push(ChangedRange::new(file, start.max(1), start.max(1)));
        } else {
            ranges.push(ChangedRange::new(file, start, start + count - 1));
        }
    }

    ranges
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

    /// A 3-symbol file: `a` on 1-10, `b` on 20-30, `c` on 40-50.
    fn three_symbol_file() -> (ArborGraph, NodeId, NodeId, NodeId) {
        let mut graph = ArborGraph::new();
        let a = graph
            .add_node(CodeNode::new("a", "a", NodeKind::Function, "src/lib.rs").with_lines(1, 10));
        let b = graph
            .add_node(CodeNode::new("b", "b", NodeKind::Function, "src/lib.rs").with_lines(20, 30));
        let c = graph
            .add_node(CodeNode::new("c", "c", NodeKind::Function, "src/lib.rs").with_lines(40, 50));
        (graph, a, b, c)
    }

    /// `mod mac` (1-40) holding `capture` (5-15) and `finish` (20-30), plus a
    /// struct (45-50) with no member nodes, and a function with a nested
    /// closure node.
    fn container_file() -> (ArborGraph, NodeId, NodeId, NodeId, NodeId, NodeId) {
        let mut graph = ArborGraph::new();
        let file = "src/lib.rs";
        let module =
            graph.add_node(CodeNode::new("mac", "mac", NodeKind::Module, file).with_lines(1, 40));
        let capture = graph.add_node(
            CodeNode::new("capture", "mac.capture", NodeKind::Function, file).with_lines(5, 15),
        );
        let finish = graph.add_node(
            CodeNode::new("finish", "mac.finish", NodeKind::Function, file).with_lines(20, 30),
        );
        let config = graph
            .add_node(CodeNode::new("Config", "Config", NodeKind::Struct, file).with_lines(45, 50));
        let outer = graph
            .add_node(CodeNode::new("outer", "outer", NodeKind::Function, file).with_lines(60, 80));
        graph.add_node(
            CodeNode::new("handler", "outer.handler", NodeKind::Function, file).with_lines(65, 70),
        );
        (graph, module, capture, finish, config, outer)
    }

    fn changed(graph: &ArborGraph, ranges: &[(u32, u32)]) -> Vec<NodeId> {
        let ranges: Vec<ChangedRange> = ranges
            .iter()
            .map(|&(s, e)| ChangedRange::new("src/lib.rs", s, e))
            .collect();
        let mut ids = changed_node_ids_for_ranges(graph, &ranges, Path::new(".")).node_ids;
        ids.sort();
        ids
    }

    #[test]
    fn an_edit_inside_a_member_does_not_change_its_container() {
        let (graph, _module, capture, finish, _, _) = container_file();
        assert_eq!(changed(&graph, &[(8, 9)]), vec![capture]);
        let mut both = vec![capture, finish];
        both.sort();
        assert_eq!(changed(&graph, &[(8, 9), (25, 25)]), both);
    }

    #[test]
    fn an_edit_outside_every_member_changes_the_container() {
        let (graph, module, capture, _, config, _) = container_file();
        // The module header, or a line between members.
        assert_eq!(changed(&graph, &[(1, 1)]), vec![module]);
        assert_eq!(changed(&graph, &[(17, 17)]), vec![module]);
        // One range across a member and the module's own lines: both.
        let mut both = vec![module, capture];
        both.sort();
        assert_eq!(changed(&graph, &[(14, 17)]), both);
        // A struct field: no member nodes, so the struct itself changed.
        assert_eq!(changed(&graph, &[(47, 47)]), vec![config]);
    }

    #[test]
    fn a_function_still_changes_when_its_closure_does() {
        let (graph, _, _, _, _, outer) = container_file();
        let ids = changed(&graph, &[(66, 66)]);
        assert!(
            ids.contains(&outer),
            "functions are not containers: {ids:?}"
        );
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn coverage_merges_adjacent_and_overlapping_spans() {
        assert!(covered(5, 15, &[(5, 10), (11, 15)]));
        assert!(covered(5, 15, &[(1, 12), (8, 20)]));
        assert!(!covered(5, 15, &[(5, 10), (12, 15)]));
        assert!(!covered(5, 15, &[]));
    }

    #[test]
    fn ranges_select_only_the_edited_symbol() {
        let (graph, _a, b, _c) = three_symbol_file();
        let ranges = vec![ChangedRange::new("src/lib.rs", 22, 23)];

        let changed = changed_node_ids_for_ranges(&graph, &ranges, Path::new("."));
        assert_eq!(
            changed.node_ids,
            vec![b],
            "editing inside b must not implicate a or c"
        );
        assert!(changed.files_without_symbol_hits.is_empty());
    }

    #[test]
    fn file_granularity_overstates_by_the_symbol_count() {
        // The regression this whole API exists to prevent.
        let (graph, _, _, _) = three_symbol_file();
        let by_file = changed_node_ids(&graph, &["src/lib.rs".to_string()], Path::new("."));
        let by_range = changed_node_ids_for_ranges(
            &graph,
            &[ChangedRange::new("src/lib.rs", 22, 23)],
            Path::new("."),
        );

        assert_eq!(by_file.len(), 3);
        assert_eq!(by_range.node_ids.len(), 1);
    }

    #[test]
    fn range_spanning_two_symbols_selects_both() {
        let (graph, _a, b, c) = three_symbol_file();
        let ranges = vec![ChangedRange::new("src/lib.rs", 25, 45)];

        let changed = changed_node_ids_for_ranges(&graph, &ranges, Path::new("."));
        assert_eq!(changed.node_ids, vec![b, c]);
    }

    #[test]
    fn boundary_lines_count_as_overlap() {
        let (graph, a, _b, _c) = three_symbol_file();

        // Touching exactly the first and last line of `a`.
        for line in [1u32, 10u32] {
            let changed = changed_node_ids_for_ranges(
                &graph,
                &[ChangedRange::new("src/lib.rs", line, line)],
                Path::new("."),
            );
            assert_eq!(changed.node_ids, vec![a], "line {line} should hit a");
        }
    }

    #[test]
    fn change_between_symbols_reports_no_symbol_hit() {
        let (graph, _, _, _) = three_symbol_file();
        // Line 15 sits in the gap between `a` and `b` — an import or comment.
        let changed = changed_node_ids_for_ranges(
            &graph,
            &[ChangedRange::new("src/lib.rs", 15, 15)],
            Path::new("."),
        );

        assert!(changed.node_ids.is_empty());
        assert_eq!(
            changed.files_without_symbol_hits,
            vec!["src/lib.rs".to_string()],
            "unmatched changes must be surfaced, not silently dropped"
        );
    }

    #[test]
    fn unpositioned_nodes_fall_back_to_file_granularity() {
        let mut graph = ArborGraph::new();
        // line_end == 0: some fallback parsers do not record positions.
        let id = graph.add_node(CodeNode::new("x", "x", NodeKind::Function, "src/lib.rs"));

        let changed = changed_node_ids_for_ranges(
            &graph,
            &[ChangedRange::new("src/lib.rs", 999, 999)],
            Path::new("."),
        );
        assert_eq!(changed.node_ids, vec![id]);
    }

    #[test]
    fn ranges_in_other_files_are_ignored() {
        let (graph, _a, _b, _c) = three_symbol_file();
        let changed = changed_node_ids_for_ranges(
            &graph,
            &[ChangedRange::new("src/other.rs", 1, 100)],
            Path::new("."),
        );
        assert!(changed.node_ids.is_empty());
        assert_eq!(changed.files_without_symbol_hits, vec!["src/other.rs"]);
    }

    #[test]
    fn parses_standard_hunk_headers() {
        let patch = "\
@@ -12,7 +12,9 @@ fn context()
-old
+new
@@ -40,3 +42,3 @@
 ctx";
        let ranges = parse_unified_diff_ranges(patch, "src/lib.rs");
        assert_eq!(
            ranges,
            vec![
                ChangedRange::new("src/lib.rs", 12, 20),
                ChangedRange::new("src/lib.rs", 42, 44),
            ]
        );
    }

    #[test]
    fn parses_hunk_header_with_omitted_count() {
        // `+5` with no comma means exactly one line.
        let ranges = parse_unified_diff_ranges("@@ -5 +5 @@\n+x", "a.rs");
        assert_eq!(ranges, vec![ChangedRange::new("a.rs", 5, 5)]);
    }

    #[test]
    fn pure_deletion_anchors_on_insertion_point() {
        // `+7,0`: the new file has no lines here, but the deletion still
        // changes whatever symbol surrounds line 7.
        let ranges = parse_unified_diff_ranges("@@ -7,3 +7,0 @@", "a.rs");
        assert_eq!(ranges, vec![ChangedRange::new("a.rs", 7, 7)]);
    }

    #[test]
    fn malformed_headers_are_skipped_not_fatal() {
        let patch = "\
@@ garbage @@
@@ -1,2 +1,2 @@
+ok
@@ -x,y +z,w @@";
        let ranges = parse_unified_diff_ranges(patch, "a.rs");
        assert_eq!(ranges, vec![ChangedRange::new("a.rs", 1, 2)]);
    }

    #[test]
    fn empty_patch_yields_no_ranges() {
        assert!(parse_unified_diff_ranges("", "a.rs").is_empty());
        assert!(parse_unified_diff_ranges("no hunks here", "a.rs").is_empty());
    }

    #[test]
    fn end_to_end_patch_to_symbols() {
        let (graph, _a, b, _c) = three_symbol_file();
        // A hunk covering lines 21-24 — inside `b` only.
        let patch = "@@ -21,4 +21,4 @@\n-old\n+new";
        let ranges = parse_unified_diff_ranges(patch, "src/lib.rs");
        let changed = changed_node_ids_for_ranges(&graph, &ranges, Path::new("."));
        assert_eq!(changed.node_ids, vec![b]);
    }
}
