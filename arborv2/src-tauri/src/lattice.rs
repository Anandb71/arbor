use crate::models::{
    DashboardSnapshot, GraphStatsView, ImpactNode, ImpactReport, IndexSummary, NodeSummary,
    PathReport, SearchResponse,
};
use anyhow::{bail, Context, Result};
use arbor_core::{CodeNode, NodeKind};
use arbor_graph::{compute_centrality, ArborGraph, GraphStore, NodeId};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

const DEFAULT_SAMPLE_QUERIES: &[&str] = &[
    "Startup Grade Lattice",
    "Operator Loop",
    "Deterministic Context",
    "Launch Readiness",
];

const DEFAULT_SEED_NOTES: &[(&str, &str)] = &[
    (
        "goals.md",
        "# Arbor V2 Mission

## Startup Grade Lattice
Build a deterministic desktop operating surface for graph-native AI work. Link execution back to [[Operator Loop]] and [[Launch Readiness]] so every surface stays grounded in proof.

## Deterministic Context
Replace flattened retrieval with explicit traversal through Arbor-backed sections. The guiding principle is that [[Core Habits]] should shape product behavior as much as infrastructure.
",
    ),
    (
        "habits.md",
        "# Core Habits

## Daily Reflection
Close each loop with evidence, not vibes alone. Feed findings into [[Launch Readiness]] and [[Deterministic Context]].

## Operator Loop
Discovery, planning, execution, and verification all happen against concrete graph state. This loop supports [[Startup Grade Lattice]] and anchors feature work to the repo.
",
    ),
    (
        "product.md",
        "# Product Surface

## Launch Readiness
The app is ready when search, impact, and pathfinding are stable, fast, and visually sharp. Tie every release gate back to [[Startup Grade Lattice]] and [[Daily Reflection]].

## Founder Dashboard
Operators need a living cockpit for graph stats, central nodes, and causal routes. That dashboard should inherit constraints from [[Deterministic Context]].
",
    ),
    (
        "ideas.md",
        "# Experiments

## Risk Gate
Use blast radius scoring to stop unsafe merges before they land. This depends on [[Launch Readiness]] and the same graph primitives used by [[Operator Loop]].

## Memory Flywheel
Every indexed note should become a reusable node in a compounding knowledge graph. That makes [[Founder Dashboard]] more useful over time.
",
    ),
];

#[derive(Debug, Default, Serialize, Deserialize)]
struct WorkspaceStateFile {
    active_notes_path: Option<String>,
}

#[derive(Debug)]
struct SectionDraft {
    title: String,
    start_line: u32,
    end_line: u32,
    signature: String,
    references: BTreeSet<String>,
}

pub struct AppState {
    runtime_root: PathBuf,
    db_path: PathBuf,
    state_path: PathBuf,
    active_notes_path: RwLock<PathBuf>,
    last_index_summary: RwLock<Option<IndexSummary>>,
}

impl AppState {
    pub fn new() -> Result<Self> {
        let runtime_root = resolve_runtime_root()?;
        fs::create_dir_all(&runtime_root)
            .with_context(|| format!("Failed to create {}", runtime_root.display()))?;

        let default_notes_path = runtime_root.join("notes");
        fs::create_dir_all(&default_notes_path)
            .with_context(|| format!("Failed to create {}", default_notes_path.display()))?;

        let state_path = runtime_root.join("workspace-state.json");
        let persisted = read_workspace_state(&state_path).unwrap_or_default();
        let active_notes_path = persisted
            .active_notes_path
            .map(PathBuf::from)
            .unwrap_or(default_notes_path);

        Ok(Self {
            db_path: runtime_root.join(".lattice.db"),
            state_path,
            runtime_root,
            active_notes_path: RwLock::new(active_notes_path),
            last_index_summary: RwLock::new(None),
        })
    }

    pub fn bootstrap(&self) -> Result<DashboardSnapshot> {
        let notes_path = self.active_notes_path()?;
        ensure_seed_notes(&notes_path, &self.runtime_root)?;
        let summary = self.index_notes_internal(&notes_path, false)?;
        self.set_last_index_summary(summary.clone())?;
        let graph = self.load_graph()?;
        Ok(self.build_dashboard(&graph, &notes_path, Some(summary)))
    }

    pub fn index_notes(&self, requested: Option<String>) -> Result<DashboardSnapshot> {
        let current = self.active_notes_path()?;
        let notes_path =
            resolve_requested_notes_path(requested.as_deref(), &self.runtime_root, &current)?;
        ensure_seed_notes(&notes_path, &self.runtime_root)?;

        let reset = normalize_cached_key(&normalize_path(&notes_path, &self.runtime_root))
            != normalize_cached_key(&normalize_path(&current, &self.runtime_root));

        let summary = self.index_notes_internal(&notes_path, reset)?;
        self.set_active_notes_path(&notes_path)?;
        self.set_last_index_summary(summary.clone())?;

        let graph = self.load_graph()?;
        Ok(self.build_dashboard(&graph, &notes_path, Some(summary)))
    }

    pub fn search_notes(&self, query: &str, limit: usize) -> Result<SearchResponse> {
        let graph = self.ensure_graph_ready()?;
        let mut candidates = collect_candidates(&graph, query, false);
        candidates.sort_by(|left, right| compare_nodes(&graph, left, right));

        let results = candidates
            .into_iter()
            .take(limit.max(1))
            .filter_map(|node_id| graph.get_by_id(&node_id))
            .map(|node| NodeSummary::from_node(&graph, node))
            .collect();

        Ok(SearchResponse {
            query: query.to_string(),
            results,
        })
    }

    pub fn analyze_note_impact(&self, query: &str, max_depth: usize) -> Result<ImpactReport> {
        let graph = self.ensure_graph_ready()?;
        let node_id = resolve_single_node(&graph, query)?;
        let analysis = graph.analyze_impact(node_id, max_depth);
        let target_node = graph
            .get(node_id)
            .context("Resolved target node but could not load it from the graph")?;

        let upstream = analysis
            .upstream
            .iter()
            .map(|affected| ImpactNode {
                node: NodeSummary::from_affected(&graph, affected),
                severity: affected.severity.to_string(),
                hop_distance: affected.hop_distance,
                direction: affected.direction.to_string(),
            })
            .collect();

        let downstream = analysis
            .downstream
            .iter()
            .map(|affected| ImpactNode {
                node: NodeSummary::from_affected(&graph, affected),
                severity: affected.severity.to_string(),
                hop_distance: affected.hop_distance,
                direction: affected.direction.to_string(),
            })
            .collect();

        Ok(ImpactReport {
            target: NodeSummary::from_node(&graph, target_node),
            summary: analysis.summary(),
            upstream,
            downstream,
            total_affected: analysis.total_affected,
            max_depth: analysis.max_depth,
            query_time_ms: analysis.query_time_ms,
        })
    }

    pub fn find_note_path(&self, from: &str, to: &str) -> Result<PathReport> {
        let graph = self.ensure_graph_ready()?;
        let from_id = resolve_single_node(&graph, from)?;
        let to_id = resolve_single_node(&graph, to)?;
        let path = graph
            .find_path(from_id, to_id)
            .with_context(|| format!("No graph path found from `{from}` to `{to}`"))?;

        let nodes = path
            .iter()
            .map(|node| NodeSummary::from_node(&graph, node))
            .collect::<Vec<_>>();

        let from_node = graph
            .get(from_id)
            .context("Resolved source node but could not read it back")?;
        let to_node = graph
            .get(to_id)
            .context("Resolved destination node but could not read it back")?;

        Ok(PathReport {
            from: NodeSummary::from_node(&graph, from_node),
            to: NodeSummary::from_node(&graph, to_node),
            nodes,
        })
    }

    fn ensure_graph_ready(&self) -> Result<ArborGraph> {
        let graph = self.load_graph()?;
        if graph.node_count() > 0 {
            return Ok(graph);
        }

        let _ = self.bootstrap()?;
        self.load_graph()
    }

    fn build_dashboard(
        &self,
        graph: &ArborGraph,
        notes_path: &Path,
        last_index_summary: Option<IndexSummary>,
    ) -> DashboardSnapshot {
        let last_index_summary = last_index_summary.or_else(|| {
            self.last_index_summary
                .read()
                .ok()
                .and_then(|summary| summary.clone())
        });
        let stats = graph.stats();
        let top_nodes = top_ranked_nodes(graph, 8);
        let mut sample_queries = top_nodes
            .iter()
            .take(4)
            .map(|node| node.name.clone())
            .collect::<Vec<_>>();

        for query in DEFAULT_SAMPLE_QUERIES {
            if sample_queries.len() >= 4 {
                break;
            }

            if !sample_queries.iter().any(|existing| existing == query) {
                sample_queries.push((*query).to_string());
            }
        }

        DashboardSnapshot {
            notes_path: display_path(notes_path, &self.runtime_root),
            db_path: display_path(&self.db_path, &self.runtime_root),
            stats: GraphStatsView {
                nodes: stats.node_count,
                edges: stats.edge_count,
                files: stats.files,
            },
            top_nodes,
            sample_queries,
            last_index_summary,
        }
    }

    fn index_notes_internal(&self, notes_path: &Path, reset: bool) -> Result<IndexSummary> {
        if reset {
            self.open_store()?
                .clear()
                .context("Failed to clear graph store")?;
        }

        let store = self.open_store()?;
        let notes_scope = normalize_cached_key(&normalize_path(notes_path, &self.runtime_root));
        let mut discovered_files = BTreeSet::new();
        let mut indexed_files = 0usize;
        let mut skipped_files = 0usize;
        let mut removed_files = 0usize;
        let mut indexed_nodes = 0usize;

        for entry in WalkDir::new(notes_path)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            if !entry.file_type().is_file() || !is_markdown_file(entry.path()) {
                continue;
            }

            let normalized_path = normalize_path(entry.path(), &self.runtime_root);
            let mtime = file_mtime_seconds(entry.path());
            discovered_files.insert(normalized_path.clone());

            if store.get_mtime(&normalized_path)? == Some(mtime) {
                skipped_files += 1;
                continue;
            }

            let source = fs::read_to_string(entry.path())
                .with_context(|| format!("Failed to read {}", entry.path().display()))?;
            let sections = parse_markdown_sections(&source, &normalized_path);

            store
                .update_file(&normalized_path, &sections, mtime)
                .with_context(|| format!("Failed to update graph store for {normalized_path}"))?;

            indexed_files += 1;
            indexed_nodes += sections.len();
        }

        for cached_file in store.list_cached_files()? {
            let normalized_cached = normalize_cached_key(&cached_file);
            if is_within_scope(&normalized_cached, &notes_scope)
                && !discovered_files.contains(&normalized_cached)
            {
                store.remove_file(&cached_file)?;
                removed_files += 1;
            }
        }

        Ok(IndexSummary {
            notes_path: display_path(notes_path, &self.runtime_root),
            db_path: display_path(&self.db_path, &self.runtime_root),
            files_indexed: indexed_files,
            files_skipped: skipped_files,
            files_removed: removed_files,
            nodes_written: indexed_nodes,
        })
    }

    fn load_graph(&self) -> Result<ArborGraph> {
        let store = self.open_store()?;
        let mut graph = store
            .load_graph()
            .context("Failed to load graph from store")?;
        let centrality = compute_centrality(&graph, 20, 0.85).into_map();
        graph.set_centrality(centrality);
        Ok(graph)
    }

    fn open_store(&self) -> Result<GraphStore> {
        GraphStore::open_or_reset(&self.db_path)
            .with_context(|| format!("Failed to open graph store at {}", self.db_path.display()))
    }

    fn active_notes_path(&self) -> Result<PathBuf> {
        self.active_notes_path
            .read()
            .map(|path| path.clone())
            .map_err(|_| anyhow::anyhow!("Notes path lock is poisoned"))
    }

    fn set_active_notes_path(&self, path: &Path) -> Result<()> {
        *self
            .active_notes_path
            .write()
            .map_err(|_| anyhow::anyhow!("Notes path lock is poisoned"))? = path.to_path_buf();

        let state = WorkspaceStateFile {
            active_notes_path: Some(path.to_string_lossy().to_string()),
        };
        let payload = serde_json::to_string_pretty(&state)?;
        fs::write(&self.state_path, payload)
            .with_context(|| format!("Failed to write {}", self.state_path.display()))?;
        Ok(())
    }

    fn set_last_index_summary(&self, summary: IndexSummary) -> Result<()> {
        *self
            .last_index_summary
            .write()
            .map_err(|_| anyhow::anyhow!("Index summary lock is poisoned"))? = Some(summary);
        Ok(())
    }
}

fn read_workspace_state(path: &Path) -> Result<WorkspaceStateFile> {
    if !path.exists() {
        return Ok(WorkspaceStateFile::default());
    }

    let payload =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", path.display()))?;
    Ok(serde_json::from_str(&payload)?)
}

fn resolve_runtime_root() -> Result<PathBuf> {
    if cfg!(debug_assertions) {
        return PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(Path::to_path_buf)
            .context("Failed to resolve Arbor V2 project root");
    }

    let data_dir = dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("ArborV2");
    Ok(data_dir)
}

fn ensure_seed_notes(notes_path: &Path, runtime_root: &Path) -> Result<()> {
    fs::create_dir_all(notes_path)
        .with_context(|| format!("Failed to create {}", notes_path.display()))?;

    let has_markdown = WalkDir::new(notes_path)
        .max_depth(1)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .any(|entry| entry.file_type().is_file() && is_markdown_file(entry.path()));

    if has_markdown {
        return Ok(());
    }

    let default_notes_path = runtime_root.join("notes");
    if notes_path != default_notes_path {
        return Ok(());
    }

    for (name, contents) in DEFAULT_SEED_NOTES {
        let note_path = notes_path.join(name);
        fs::write(&note_path, contents)
            .with_context(|| format!("Failed to seed {}", note_path.display()))?;
    }

    Ok(())
}

fn resolve_requested_notes_path(
    input: Option<&str>,
    runtime_root: &Path,
    current: &Path,
) -> Result<PathBuf> {
    let Some(raw) = input.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(current.to_path_buf());
    };

    let path = PathBuf::from(raw);
    if path.is_absolute() {
        return Ok(path);
    }

    let runtime_relative = runtime_root.join(&path);
    if runtime_relative.exists() {
        return Ok(runtime_relative);
    }

    if let Some(parent) = runtime_root.parent() {
        let parent_relative = parent.join(&path);
        if parent_relative.exists() {
            return Ok(parent_relative);
        }
    }

    Ok(runtime_relative)
}

fn parse_markdown_sections(source: &str, file_path: &str) -> Vec<CodeNode> {
    let mut sections = Vec::new();
    let mut current: Option<SectionDraft> = None;
    let mut heading_stack: Vec<String> = Vec::new();
    let mut in_fenced_block = false;
    let mut saw_heading = false;
    let total_lines = source.lines().count().max(1) as u32;

    for (index, raw_line) in source.lines().enumerate() {
        let line_no = index as u32 + 1;
        let trimmed = raw_line.trim();

        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fenced_block = !in_fenced_block;
            continue;
        }

        if in_fenced_block {
            continue;
        }

        if let Some((level, title)) = parse_heading(raw_line) {
            saw_heading = true;
            if let Some(section) = current.take() {
                sections.push(build_section_node(file_path, section));
            }

            while heading_stack.len() >= level {
                heading_stack.pop();
            }

            let mut references = BTreeSet::new();
            if let Some(parent) = heading_stack.last() {
                references.insert(parent.clone());
            }
            for reference in extract_wiki_links(title) {
                references.insert(reference);
            }

            heading_stack.push(title.to_string());
            current = Some(SectionDraft {
                title: title.to_string(),
                start_line: line_no,
                end_line: line_no,
                signature: raw_line.trim().to_string(),
                references,
            });
            continue;
        }

        if let Some(section) = current.as_mut() {
            section.end_line = line_no;
            for reference in extract_wiki_links(raw_line) {
                section.references.insert(reference);
            }
        }
    }

    if let Some(mut section) = current.take() {
        section.end_line = total_lines.max(section.start_line);
        sections.push(build_section_node(file_path, section));
    }

    if !saw_heading && !source.trim().is_empty() {
        let title = infer_note_title(file_path);
        let mut references = BTreeSet::new();
        for reference in extract_wiki_links(source) {
            references.insert(reference);
        }

        sections.push(build_section_node(
            file_path,
            SectionDraft {
                signature: format!("# {title}"),
                title,
                start_line: 1,
                end_line: total_lines,
                references,
            },
        ));
    }

    sections
}

fn build_section_node(file_path: &str, draft: SectionDraft) -> CodeNode {
    let qualified_name = format!("{file_path}::{}", draft.title);
    CodeNode::new(&draft.title, qualified_name, NodeKind::Section, file_path)
        .with_lines(draft.start_line, draft.end_line.max(draft.start_line))
        .with_signature(draft.signature)
        .with_references(draft.references.into_iter().collect())
}

fn parse_heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let level = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if !(1..=6).contains(&level) {
        return None;
    }

    let title = trimmed[level..].trim().trim_end_matches('#').trim();
    if title.is_empty() {
        None
    } else {
        Some((level, title))
    }
}

fn extract_wiki_links(text: &str) -> Vec<String> {
    let mut references = Vec::new();
    let mut remainder = text;

    while let Some(start) = remainder.find("[[") {
        let after_open = &remainder[start + 2..];
        let Some(end) = after_open.find("]]") else {
            break;
        };

        let raw_target = after_open[..end].trim();
        let target = raw_target.split('|').next().unwrap_or(raw_target).trim();
        let target = if let Some((_, section)) = target.split_once('#') {
            section.trim()
        } else {
            target
        };

        if !target.is_empty() {
            references.push(target.to_string());
        }

        remainder = &after_open[end + 2..];
    }

    references
}

fn infer_note_title(file_path: &str) -> String {
    Path::new(file_path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("note")
        .replace(['_', '-'], " ")
}

fn file_mtime_seconds(path: &Path) -> u64 {
    path.metadata()
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn is_markdown_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(extension.to_ascii_lowercase().as_str(), "md" | "markdown")
        })
}

fn normalize_path(path: &Path, root: &Path) -> String {
    let normalized = path.strip_prefix(root).unwrap_or(path);
    normalized.to_string_lossy().replace('\\', "/")
}

fn normalize_cached_key(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_string()
}

fn display_path(path: &Path, root: &Path) -> String {
    normalize_path(path, root)
}

fn is_within_scope(path: &str, scope: &str) -> bool {
    let prefix = format!("{}/", scope.trim_end_matches('/'));
    path == scope || path.starts_with(&prefix)
}

fn collect_candidates(graph: &ArborGraph, query: &str, exact_only: bool) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();

    if let Some(index) = graph.get_index(query) {
        if let Some(node) = graph.get(index) {
            if seen.insert(node.id.clone()) {
                out.push(node.id.clone());
            }
        }
    }

    for node in graph.find_by_name(query) {
        if seen.insert(node.id.clone()) {
            out.push(node.id.clone());
        }
    }

    if !exact_only {
        for node in graph.search(query) {
            if seen.insert(node.id.clone()) {
                out.push(node.id.clone());
            }
        }
    }

    out
}

fn resolve_single_node(graph: &ArborGraph, query: &str) -> Result<NodeId> {
    let mut candidates = collect_candidates(graph, query, false);
    candidates.sort_by(|left, right| compare_nodes(graph, left, right));

    match candidates.as_slice() {
        [] => bail!("Could not resolve `{query}` to any indexed section."),
        [node_id] => graph
            .get_index(node_id)
            .with_context(|| format!("Resolved `{query}` but failed to recover node id.")),
        many => {
            let options = many
                .iter()
                .take(5)
                .map(|node_id| format!("  - {}", format_node(graph, node_id)))
                .collect::<Vec<_>>()
                .join("\n");
            bail!(
                "Query `{query}` is ambiguous. Narrow it by using the exact section title or node id.\n{options}"
            )
        }
    }
}

fn compare_nodes(graph: &ArborGraph, left_id: &str, right_id: &str) -> Ordering {
    let left_score = node_score(graph, left_id);
    let right_score = node_score(graph, right_id);

    right_score
        .partial_cmp(&left_score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            let left = graph.get_by_id(left_id);
            let right = graph.get_by_id(right_id);
            match (left, right) {
                (Some(left), Some(right)) => left
                    .name
                    .cmp(&right.name)
                    .then_with(|| left.file.cmp(&right.file))
                    .then_with(|| left.id.cmp(&right.id)),
                _ => left_id.cmp(right_id),
            }
        })
}

fn node_score(graph: &ArborGraph, node_id: &str) -> f64 {
    graph
        .get_index(node_id)
        .map(|index| graph.centrality(index))
        .unwrap_or(0.0)
}

fn top_ranked_nodes(graph: &ArborGraph, limit: usize) -> Vec<NodeSummary> {
    let mut node_ids = graph
        .nodes()
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    node_ids.sort_by(|left, right| compare_nodes(graph, left, right));

    node_ids
        .into_iter()
        .take(limit)
        .filter_map(|node_id| graph.get_by_id(&node_id))
        .map(|node| NodeSummary::from_node(graph, node))
        .collect()
}

fn format_node(graph: &ArborGraph, node_id: &str) -> String {
    let Some(node) = graph.get_by_id(node_id) else {
        return node_id.to_string();
    };

    let score = node_score(graph, node_id);
    let line = if node.line_start > 0 {
        format!(":{}", node.line_start)
    } else {
        String::new()
    };

    format!(
        "{} [{}] {}{} score {:.2}",
        node.name, node.kind, node.file, line, score
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_markdown_sections_preserves_titles_and_parent_links() {
        let sections =
            parse_markdown_sections("# Mission\n\n## Build Arbor V2\n", "notes/goals.md");

        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].name, "Mission");
        assert_eq!(sections[1].name, "Build Arbor V2");
        assert!(sections[1]
            .references
            .iter()
            .any(|reference| reference == "Mission"));
    }

    #[test]
    fn parse_markdown_sections_extracts_wiki_links() {
        let sections = parse_markdown_sections(
            "# Core Habits\n\nFocus on [[Daily Reflection]] and [[ideas#Risk Gate]].\n",
            "notes/habits.md",
        );

        assert_eq!(sections.len(), 1);
        assert!(sections[0]
            .references
            .iter()
            .any(|reference| reference == "Daily Reflection"));
        assert!(sections[0]
            .references
            .iter()
            .any(|reference| reference == "Risk Gate"));
    }

    #[test]
    fn resolve_requested_notes_path_prefers_runtime_relative_paths() {
        let runtime_root = Path::new("C:/repo/arborv2");
        let current = runtime_root.join("notes");

        let resolved = resolve_requested_notes_path(Some("notes"), runtime_root, &current).unwrap();

        assert_eq!(resolved, current);
    }
}
