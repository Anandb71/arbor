//! CLI command implementations.

use arbor_core::parse_file;
use arbor_graph::compute_centrality;
use arbor_server::{ArborServer, ServerConfig};
use arbor_watcher::{index_directory, IndexOptions};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug)]
struct DiffSummary {
    changed_files: Vec<String>,
    changed_symbols: usize,
    direct_callers: usize,
    indirect_callers: usize,
    entrypoints_affected: usize,
    files_likely_updates: usize,
    blast_radius_nodes: usize,
}

const ROOT_MARKERS: &[&str] = &[
    ".arbor",
    ".git",
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "pubspec.yaml",
];

fn find_workspace_root(start: &Path) -> PathBuf {
    let mut current = fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    if current.is_file() {
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        }
    }

    let mut best = current.clone();
    loop {
        if ROOT_MARKERS
            .iter()
            .any(|marker| current.join(marker).exists())
        {
            best = current.clone();
        }

        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }

    best
}

fn resolve_project_path(path: &Path) -> Result<PathBuf> {
    let base = if path == Path::new(".") {
        std::env::current_dir()?
    } else {
        path.to_path_buf()
    };
    Ok(find_workspace_root(&base))
}

fn ensure_arbor_initialized(path: &Path) -> Result<bool> {
    let arbor_dir = path.join(".arbor");
    let config_path = arbor_dir.join("config.json");

    if !arbor_dir.exists() {
        fs::create_dir_all(&arbor_dir)?;
    }

    if !config_path.exists() {
        let default_config = serde_json::json!({
            "version": "1.0",
            "languages": [
                "typescript",
                "javascript",
                "rust",
                "python",
                "go",
                "java",
                "c",
                "cpp",
                "csharp",
                "dart"
            ],
            "ignore": ["node_modules", "target", "dist", "__pycache__", ".venv", "build", "out"]
        });
        fs::write(&config_path, serde_json::to_string_pretty(&default_config)?)?;
        return Ok(true);
    }

    Ok(false)
}

fn graph_snapshot_path(path: &Path) -> PathBuf {
    path.join(".arbor").join("graph.json")
}

fn graph_binary_path(path: &Path) -> PathBuf {
    path.join(".arbor").join("graph.bin")
}

fn graph_store_path(path: &Path) -> PathBuf {
    path.join(".arbor").join("cache")
}

fn save_graph_snapshot(path: &Path, graph: &arbor_graph::ArborGraph) -> Result<()> {
    let graph_path = graph_snapshot_path(path);
    if let Some(parent) = graph_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = std::fs::File::create(&graph_path)?;
    let writer = std::io::BufWriter::new(file);
    serde_json::to_writer_pretty(writer, graph)?;
    Ok(())
}

fn save_graph_binary(path: &Path, graph: &arbor_graph::ArborGraph) -> Result<()> {
    let graph_path = graph_binary_path(path);
    if let Some(parent) = graph_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let bytes = bincode::serialize(graph)?;
    fs::write(graph_path, bytes)?;
    Ok(())
}

fn load_graph_snapshot(path: &Path) -> Result<arbor_graph::ArborGraph> {
    let graph_path = graph_snapshot_path(path);

    if !graph_path.exists() {
        return Err(format!(
            "Graph not found at {}. Run 'arbor index' first.",
            graph_path.display()
        )
        .into());
    }

    let file = std::fs::File::open(&graph_path)?;
    let reader = std::io::BufReader::new(file);
    let graph: arbor_graph::ArborGraph = serde_json::from_reader(reader)?;
    Ok(graph)
}

fn load_graph_binary(path: &Path) -> Result<arbor_graph::ArborGraph> {
    let graph_path = graph_binary_path(path);
    if !graph_path.exists() {
        return Err(format!("Binary graph not found at {}", graph_path.display()).into());
    }

    let bytes = fs::read(graph_path)?;
    let graph: arbor_graph::ArborGraph = bincode::deserialize(&bytes)?;
    Ok(graph)
}

fn load_graph_from_store(path: &Path) -> Result<arbor_graph::ArborGraph> {
    let store_path = graph_store_path(path);
    if !store_path.exists() {
        return Err("No graph store cache found".into());
    }

    let store = arbor_graph::GraphStore::open_or_reset(&store_path)
        .map_err(|e| format!("Failed to open graph store: {}", e))?;

    let graph = store
        .load_graph()
        .map_err(|e| format!("Failed to load graph from store: {}", e))?;

    if graph.node_count() == 0 {
        return Err("Graph store was empty".into());
    }

    Ok(graph)
}

fn load_or_index_graph(path: &Path) -> Result<arbor_graph::ArborGraph> {
    if let Ok(graph) = load_graph_binary(path) {
        return Ok(graph);
    }

    if let Ok(graph) = load_graph_snapshot(path) {
        return Ok(graph);
    }

    if let Ok(graph) = load_graph_from_store(path) {
        // Materialize JSON snapshot for faster future warm starts.
        let _ = save_graph_snapshot(path, &graph);
        let _ = save_graph_binary(path, &graph);
        return Ok(graph);
    }

    let result = index_directory(path, IndexOptions::default())?;
    save_graph_snapshot(path, &result.graph)?;
    save_graph_binary(path, &result.graph)?;
    Ok(result.graph)
}

fn run_git(path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).current_dir(path).output()?;
    if !output.status.success() {
        return Err(format!("git {:?} failed", args).into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn is_git_repo(path: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn parse_git_name_status_output(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                return None;
            }

            let parts: Vec<&str> = trimmed.split('\t').collect();
            if parts.len() < 2 {
                return None;
            }

            let status = parts[0];
            let path = if status.starts_with('R') || status.starts_with('C') {
                parts.get(2).copied().unwrap_or(parts[1])
            } else if status.starts_with('D') {
                return None;
            } else {
                parts[1]
            };

            let normalized = normalize_slashes(path.trim());
            if normalized.is_empty() {
                None
            } else {
                Some(normalized)
            }
        })
        .collect()
}

fn is_generated_or_internal_path(path: &str) -> bool {
    let normalized = normalize_slashes(path).to_lowercase();
    let with_boundary = format!("/{normalized}/");

    if normalized.starts_with(".arbor/") || with_boundary.contains("/.arbor/") {
        return true;
    }

    ["target", "dist", "build", ".dart_tool", "generated"]
        .iter()
        .any(|segment| with_boundary.contains(&format!("/{segment}/")))
        || [".g.dart", ".generated.rs", ".pb.go", ".designer.cs"]
            .iter()
            .any(|suffix| normalized.ends_with(suffix))
}

fn git_changed_files(path: &Path) -> Result<Vec<String>> {
    if !is_git_repo(path) {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();

    let unstaged = run_git(
        path,
        &["diff", "-w", "--name-status", "--find-renames", "HEAD"],
    )?;
    files.extend(parse_git_name_status_output(&unstaged));

    let staged = run_git(
        path,
        &[
            "diff",
            "--cached",
            "-w",
            "--name-status",
            "--find-renames",
            "HEAD",
        ],
    )?;
    files.extend(parse_git_name_status_output(&staged));

    let untracked = run_git(path, &["ls-files", "--others", "--exclude-standard"])?;
    files.extend(
        untracked
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(normalize_slashes),
    );

    files.retain(|path| !is_generated_or_internal_path(path));
    files.sort();
    files.dedup();
    Ok(files)
}

fn normalize_slashes(input: &str) -> String {
    input.replace('\\', "/")
}

fn node_matches_changed_file(node_file: &str, changed_file: &str, project_root: &Path) -> bool {
    let node_norm = normalize_slashes(node_file);
    let changed_norm = normalize_slashes(changed_file);

    if node_norm.ends_with(&changed_norm) {
        return true;
    }

    let abs = project_root.join(&changed_norm);
    let abs_norm = normalize_slashes(&abs.to_string_lossy());
    node_norm == abs_norm
}

fn changed_node_ids(
    graph: &arbor_graph::ArborGraph,
    changed_files: &[String],
    project_root: &Path,
) -> Vec<arbor_graph::NodeId> {
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

fn compute_diff_summary(
    graph: &arbor_graph::ArborGraph,
    changed_files: Vec<String>,
    changed_node_ids: Vec<arbor_graph::NodeId>,
    max_depth: usize,
    project_root: &Path,
) -> DiffSummary {
    let mut direct_callers = std::collections::HashSet::new();
    let mut indirect_callers = std::collections::HashSet::new();
    let mut affected_nodes = std::collections::HashSet::new();
    let mut affected_files = std::collections::HashSet::new();

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

    DiffSummary {
        changed_files,
        changed_symbols: changed_node_ids.len(),
        direct_callers: direct_callers.len(),
        indirect_callers: indirect_callers.len(),
        entrypoints_affected,
        files_likely_updates,
        blast_radius_nodes: affected_nodes.len(),
    }
}

fn print_diff_summary(summary: &DiffSummary) {
    println!("{}", "Change Impact Preview".cyan().bold());
    println!();
    println!("Modified files:");
    for f in &summary.changed_files {
        println!("  • {}", f);
    }
    println!();
    println!("Impact:");
    println!("  • {} direct callers", summary.direct_callers);
    println!("  • {} indirect callers", summary.indirect_callers);
    println!(
        "  • {} API entrypoints affected",
        summary.entrypoints_affected
    );
    println!(
        "  • {} files likely require updates",
        summary.files_likely_updates
    );
    println!("  • {} impacted nodes total", summary.blast_radius_nodes);
    println!("  • {} changed symbols resolved", summary.changed_symbols);
}

fn resolve_node_or_file_target(
    graph: &arbor_graph::ArborGraph,
    symbol: &str,
    project_root: &Path,
) -> Option<(String, u32)> {
    let candidate_path = project_root.join(symbol);
    if candidate_path.exists() {
        return Some((candidate_path.to_string_lossy().to_string(), 1));
    }

    if let Some(idx) = graph.get_index(symbol) {
        if let Some(node) = graph.get(idx) {
            return Some((node.file.clone(), node.line_start));
        }
    }

    graph
        .find_by_name(symbol)
        .first()
        .map(|node| (node.file.clone(), node.line_start))
}

fn command_exists(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn open_in_editor(file: &str, line: u32) -> Result<()> {
    let editor = std::env::var("ARBOR_EDITOR").ok();
    let targets = if let Some(e) = editor {
        vec![e]
    } else {
        vec![
            "cursor".to_string(),
            "code".to_string(),
            "nvim".to_string(),
            "vim".to_string(),
        ]
    };

    for cmd in targets {
        if !command_exists(&cmd) {
            continue;
        }

        let status = if cmd == "cursor" || cmd == "code" {
            Command::new(&cmd)
                .arg("-g")
                .arg(format!("{}:{}", file, line))
                .status()?
        } else {
            Command::new(&cmd)
                .arg(format!("+{}", line))
                .arg(file)
                .status()?
        };

        if status.success() {
            return Ok(());
        }
    }

    Err("No supported editor found (cursor/code/nvim/vim). Set ARBOR_EDITOR to override.".into())
}

/// Initialize Arbor in a directory.
pub fn init(path: &Path) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let arbor_dir = resolved_path.join(".arbor");

    if arbor_dir.exists() {
        println!(
            "{} Already initialized at {}",
            "✓".green(),
            resolved_path.display()
        );
        return Ok(());
    }

    let _ = ensure_arbor_initialized(&resolved_path)?;

    println!(
        "{} Initialized Arbor in {}",
        "✓".green(),
        resolved_path.display()
    );
    println!("  Run {} to index your codebase", "arbor index".cyan());

    Ok(())
}

/// Index a directory and build the code graph.
pub fn index(
    path: &Path,
    output: Option<&Path>,
    follow_symlinks: bool,
    no_cache: bool,
    changed_only: bool,
) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let was_initialized = ensure_arbor_initialized(&resolved_path)?;
    if was_initialized {
        println!(
            "{} Created {} for first-time setup",
            "✓".green(),
            resolved_path.join(".arbor").display()
        );
    }

    if changed_only {
        return index_changed_only(&resolved_path, output, follow_symlinks);
    }

    println!("{}", "Indexing codebase...".cyan());

    let spinner = ProgressBar::new_spinner();
    spinner.set_style(ProgressStyle::default_spinner().template("{spinner:.cyan} {msg}")?);
    spinner.enable_steady_tick(Duration::from_millis(80));
    spinner.set_message("Scanning files...");

    // Determine cache path
    let cache_path = if no_cache {
        None
    } else {
        Some(resolved_path.join(".arbor").join("cache"))
    };

    let options = IndexOptions {
        follow_symlinks,
        cache_path,
    };
    let result = index_directory(&resolved_path, options)?;

    spinner.finish_and_clear();

    // Print results
    let cache_msg = if result.cache_hits > 0 {
        format!(" ({} from cache)", result.cache_hits)
    } else {
        String::new()
    };
    println!(
        "{} Indexed {} files{} ({} nodes) in {}ms",
        "✓".green(),
        result.files_indexed.to_string().cyan(),
        cache_msg.dimmed(),
        result.nodes_extracted.to_string().cyan(),
        result.duration_ms
    );

    // Warn if graph is empty
    if result.nodes_extracted == 0 {
        eprintln!("\n{} No nodes extracted. Check:", "⚠ Warning:".yellow());
        eprintln!("  - File extensions match supported languages (.rs, .ts, .py, .dart, .go, .kt, .swift, .rb, .php, .sh)");
        eprintln!("  - Path is not excluded by .gitignore");
        eprintln!("  - Files contain parseable function/class definitions");
    }

    // Show any errors
    if !result.errors.is_empty() {
        println!("\n{} files with parse errors:", "⚠".yellow());
        for (file, error) in result.errors.iter().take(5) {
            println!("  {} - {}", file.red(), error);
        }
        if result.errors.len() > 5 {
            println!("  ... and {} more", result.errors.len() - 5);
        }
    }

    // Export if requested
    if let Some(out_path) = output {
        export_graph(&result.graph, out_path)?;
    }

    save_graph_snapshot(&resolved_path, &result.graph)?;
    save_graph_binary(&resolved_path, &result.graph)?;
    println!(
        "{} Saved graph snapshot to {}",
        "✓".green(),
        graph_snapshot_path(&resolved_path).display()
    );

    Ok(())
}

fn index_changed_only(path: &Path, output: Option<&Path>, follow_symlinks: bool) -> Result<()> {
    let changed_files = git_changed_files(path)?;
    if changed_files.is_empty() {
        println!(
            "{} No git changes detected. Nothing to re-index.",
            "✓".green()
        );
        return Ok(());
    }

    println!(
        "{} Incremental indexing (changed files only)...",
        "⚡".cyan()
    );
    let base_graph = load_or_index_graph(path)?;

    let mut retained_nodes = Vec::new();
    for node in base_graph.nodes() {
        let changed = changed_files
            .iter()
            .any(|f| node_matches_changed_file(&node.file, f, path));
        if !changed {
            retained_nodes.push(node.clone());
        }
    }

    let mut parsed_nodes = Vec::new();
    let mut parsed_files = 0usize;
    let mut parse_errors = 0usize;

    for rel in &changed_files {
        let abs = path.join(rel);
        if !abs.exists() {
            continue; // deleted file, already removed by retained_nodes filter
        }
        if abs.is_dir() {
            continue;
        }

        let extension = match abs.extension().and_then(|e| e.to_str()) {
            Some(ext) => ext,
            None => continue,
        };

        if !arbor_core::languages::is_supported(extension) {
            continue;
        }

        match parse_file(&abs) {
            Ok(nodes) => {
                parsed_files += 1;
                parsed_nodes.extend(nodes);
            }
            Err(_) => {
                parse_errors += 1;
            }
        }
    }

    let mut builder = arbor_graph::GraphBuilder::new();
    builder.add_nodes(retained_nodes);
    builder.add_nodes(parsed_nodes);
    let graph = builder.build();

    save_graph_snapshot(path, &graph)?;
    save_graph_binary(path, &graph)?;

    if let Some(out_path) = output {
        export_graph(&graph, out_path)?;
    }

    println!(
        "{} Incremental index done: {} changed files parsed, {} parse errors, {} total nodes",
        "✓".green(),
        parsed_files,
        parse_errors,
        graph.node_count()
    );
    println!(
        "{} Follow symlinks mode: {}",
        "ℹ".blue(),
        if follow_symlinks { "on" } else { "off" }
    );

    Ok(())
}

fn export_graph(graph: &arbor_graph::ArborGraph, path: &Path) -> Result<()> {
    let nodes: Vec<_> = graph.nodes().collect();

    let export = serde_json::json!({
        "version": "1.0",
        "stats": {
            "nodeCount": graph.node_count(),
            "edgeCount": graph.edge_count()
        },
        "nodes": nodes
    });

    fs::write(path, serde_json::to_string_pretty(&export)?)?;
    println!("{} Exported to {}", "✓".green(), path.display());

    Ok(())
}

pub fn query(query: &str, limit: usize, path: &Path) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    let matches: Vec<_> = graph.search(query).into_iter().take(limit).collect();

    if matches.is_empty() {
        println!("No matches found for \"{}\"", query);
        return Ok(());
    }

    println!("Found {} matches:\n", matches.len());

    for node in matches {
        println!(
            "  {} {} {}",
            node.kind.to_string().yellow(),
            node.qualified_name.cyan(),
            format!("({}:{})", node.file, node.line_start).dimmed()
        );
        if let Some(ref sig) = node.signature {
            println!("    {}", sig.dimmed());
        }
    }

    Ok(())
}

pub fn diff(path: &Path, depth: usize, json_output: bool) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;

    if !is_git_repo(&resolved_path) {
        return Err("arbor diff requires a git repository".into());
    }

    let changed_files = git_changed_files(&resolved_path)?;
    if changed_files.is_empty() {
        println!("{} No modified files detected against HEAD.", "✓".green());
        return Ok(());
    }

    let graph = load_or_index_graph(&resolved_path)?;
    let changed_nodes = changed_node_ids(&graph, &changed_files, &resolved_path);
    let summary = compute_diff_summary(&graph, changed_files, changed_nodes, depth, &resolved_path);

    if json_output {
        let output = serde_json::json!({
            "changed_files": summary.changed_files,
            "changed_symbols": summary.changed_symbols,
            "impact": {
                "direct_callers": summary.direct_callers,
                "indirect_callers": summary.indirect_callers,
                "api_entrypoints_affected": summary.entrypoints_affected,
                "files_likely_require_updates": summary.files_likely_updates,
                "blast_radius_nodes": summary.blast_radius_nodes
            }
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    print_diff_summary(&summary);
    Ok(())
}

pub fn check(
    path: &Path,
    depth: usize,
    max_blast_radius: usize,
    no_fail: bool,
    json_output: bool,
) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;

    if !is_git_repo(&resolved_path) {
        return Err("arbor check requires a git repository".into());
    }

    let changed_files = git_changed_files(&resolved_path)?;
    let graph = load_or_index_graph(&resolved_path)?;
    let changed_nodes = changed_node_ids(&graph, &changed_files, &resolved_path);
    let summary = compute_diff_summary(&graph, changed_files, changed_nodes, depth, &resolved_path);

    let risky = summary.blast_radius_nodes > max_blast_radius
        || summary.entrypoints_affected > 0
        || summary.indirect_callers > max_blast_radius / 2;

    if json_output {
        let output = serde_json::json!({
            "risky": risky,
            "thresholds": {
                "max_blast_radius": max_blast_radius
            },
            "summary": {
                "changed_files": summary.changed_files,
                "changed_symbols": summary.changed_symbols,
                "direct_callers": summary.direct_callers,
                "indirect_callers": summary.indirect_callers,
                "api_entrypoints_affected": summary.entrypoints_affected,
                "files_likely_require_updates": summary.files_likely_updates,
                "blast_radius_nodes": summary.blast_radius_nodes
            }
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else if risky {
        println!("{}", "High risk refactor detected.".red().bold());
        println!();
        print_diff_summary(&summary);
        println!();
        println!("Recommendation: run integration tests.");
    } else {
        println!("{}", "Safe change window detected.".green().bold());
        print_diff_summary(&summary);
    }

    if risky && !no_fail {
        return Err("risky change set detected".into());
    }

    Ok(())
}

/// Start the Arbor server.
pub async fn serve(port: u16, headless: bool, path: &Path, follow_symlinks: bool) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;
    let bind_addr = if headless { "0.0.0.0" } else { "127.0.0.1" };

    if headless {
        println!("{}", "Starting Arbor server in headless mode...".cyan());
    } else {
        println!("{}", "Starting Arbor server...".cyan());
    }

    // Index the codebase first
    let options = IndexOptions {
        follow_symlinks,
        cache_path: None,
    };
    let result = index_directory(&resolved_path, options)?;
    let mut graph = result.graph;

    // Compute centrality
    let scores = compute_centrality(&graph, 20, 0.85);
    graph.set_centrality(scores.into_map());

    println!(
        "{} Indexed {} files ({} nodes)",
        "✓".green(),
        result.files_indexed,
        result.nodes_extracted
    );

    let addr = format!("{}:{}", bind_addr, port).parse()?;
    let config = ServerConfig { addr };
    let server = ArborServer::new(graph, config);

    println!("{} Listening on ws://{}:{}", "✓".green(), bind_addr, port);
    if headless {
        println!("  Headless mode: accepting connections from any host");
    }
    println!("  Press {} to stop", "Ctrl+C".cyan());

    server.run().await.map_err(|e| e.to_string())?;

    Ok(())
}

/// Start the Arbor Visualizer.
pub async fn viz(path: &Path, follow_symlinks: bool) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;
    println!("{}", "Starting Arbor Visualizer stack...".cyan());

    // 1. Index Codebase
    let options = IndexOptions {
        follow_symlinks,
        cache_path: None,
    };
    let result = index_directory(&resolved_path, options)?;
    let mut graph = result.graph;

    // Compute centrality for better initial layout
    println!("Computing centrality...");
    let scores = compute_centrality(&graph, 20, 0.85);
    graph.set_centrality(scores.into_map());

    println!(
        "{} Indexed {} files ({} nodes)",
        "✓".green(),
        result.files_indexed,
        result.nodes_extracted
    );

    // 2. Start API Server (JSON-RPC)
    let rpc_port = 7433;
    let rpc_addr = format!("127.0.0.1:{}", rpc_port).parse()?;
    let rpc_config = ServerConfig { addr: rpc_addr };
    let arbor_server = ArborServer::new(graph, rpc_config);
    let shared_graph = arbor_server.graph();

    // 3. Start Sync Server (WebSocket Broadcast)
    let sync_port = 8081;
    let sync_addr = format!("127.0.0.1:{}", sync_port).parse()?;
    let sync_config = arbor_server::SyncServerConfig {
        addr: sync_addr,
        watch_path: resolved_path.to_path_buf(),
        debounce_ms: 1000,
        extensions: vec![
            "ts".to_string(),
            "tsx".to_string(),
            "js".to_string(),
            "jsx".to_string(),
            "rs".to_string(),
            "py".to_string(),
            "dart".to_string(),
            "go".to_string(),
            "java".to_string(),
            "c".to_string(),
            "h".to_string(),
            "cpp".to_string(),
            "hpp".to_string(),
            "cc".to_string(),
            "cxx".to_string(),
            "hh".to_string(),
            "cs".to_string(),
            "kt".to_string(),
            "kts".to_string(),
            "swift".to_string(),
            "rb".to_string(),
            "php".to_string(),
            "phtml".to_string(),
            "sh".to_string(),
            "bash".to_string(),
            "zsh".to_string(),
        ],
    };
    let sync_server = arbor_server::SyncServer::new_with_shared(sync_config, shared_graph.clone());

    // Spawn servers
    println!("{} RPC Server on port {}", "✓".green(), rpc_port);
    println!("{} Sync Server on port {}", "✓".green(), sync_port);

    tokio::spawn(async move {
        if let Err(e) = arbor_server.run().await {
            eprintln!("RPC Server error: {}", e);
        }
    });

    tokio::spawn(async move {
        if let Err(e) = sync_server.run().await {
            eprintln!("Sync Server error: {}", e);
        }
    });

    // 4. Launch Visualizer
    // Priority 1: Standalone bundled executable (relative to arbor binary)
    let current_exe = std::env::current_exe()?;
    let exe_dir = current_exe.parent().unwrap_or(&resolved_path);

    #[cfg(target_os = "windows")]
    let bundled_viz = exe_dir.join("arbor_visualizer").join("visualizer.exe");
    #[cfg(target_os = "macos")]
    let bundled_viz = exe_dir
        .join("arbor_visualizer")
        .join("arbor_visualizer.app")
        .join("Contents")
        .join("MacOS")
        .join("arbor_visualizer");
    #[cfg(target_os = "linux")]
    let bundled_viz = exe_dir.join("arbor_visualizer").join("arbor_visualizer");

    if bundled_viz.exists() {
        println!("{} Launching bundled visualizer...", "🚀".cyan());
        let status = std::process::Command::new(&bundled_viz)
            .current_dir(bundled_viz.parent().unwrap())
            .status();

        match status {
            Ok(_) => println!("Visualizer closed."),
            Err(e) => println!("Failed to launch bundled visualizer: {}", e),
        }
    } else {
        // Priority 2: Source code (Flutter dev mode)
        let viz_dir = resolved_path.join("visualizer");
        if viz_dir.exists() {
            println!("{}", "Launching Flutter Visualizer (Dev Mode)...".cyan());

            #[cfg(target_os = "windows")]
            let (cmd, device) = ("flutter.bat", "windows");
            #[cfg(target_os = "macos")]
            let (cmd, device) = ("flutter", "macos");
            #[cfg(target_os = "linux")]
            let (cmd, device) = ("flutter", "linux");

            let status = std::process::Command::new(cmd)
                .arg("run")
                .arg("-d")
                .arg(device)
                .current_dir(&viz_dir)
                .status();

            match status {
                Ok(_) => println!("Visualizer closed."),
                Err(e) => println!("Failed to launch visualizer: {}", e),
            }
        } else {
            println!(
                "{}",
                "Visualizer not found (neither bundled 'arbor_visualizer' nor source 'visualizer' detected).".yellow()
            );
            println!("Please download the full Arbor release or run from source.");
        }
    }

    Ok(())
}

/// Export the graph to JSON.
pub fn export(path: &Path, output: &Path) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;
    let result = index_directory(&resolved_path, IndexOptions::default())?;
    export_graph(&result.graph, output)?;
    Ok(())
}

/// Show index status.
pub fn status(path: &Path, show_files: bool) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let was_initialized = ensure_arbor_initialized(&resolved_path)?;
    if was_initialized {
        println!(
            "{} Auto-initialized Arbor at {}",
            "✓".green(),
            resolved_path.join(".arbor").display()
        );
    }

    // Quick index to get stats
    let result = index_directory(&resolved_path, IndexOptions::default())?;

    // Collect unique files from indexed nodes
    let files: std::collections::HashSet<_> =
        result.graph.nodes().map(|n| n.file.clone()).collect();

    // Collect unique extensions from indexed files
    let mut file_exts: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut ext_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for node in result.graph.nodes() {
        if file_exts.insert(node.file.clone()) {
            if let Some(ext) = std::path::Path::new(&node.file)
                .extension()
                .and_then(|e| e.to_str())
            {
                *ext_counts.entry(ext.to_string()).or_insert(0) += 1;
            }
        }
    }

    let mut ext_list: Vec<_> = ext_counts.iter().collect();
    // Sort by count descending
    ext_list.sort_by(|a, b| b.1.cmp(a.1));

    println!("{}", "📊 Arbor Status".cyan().bold());
    println!();
    println!("  {} {}", "Files indexed:".dimmed(), result.files_indexed);
    println!("  {} {}", "Nodes:".dimmed(), result.nodes_extracted);
    println!("  {} {}", "Edges:".dimmed(), result.graph.edge_count());

    if show_files {
        println!();
        println!("  {}", "Extensions (by file count):".yellow());
        if ext_list.is_empty() {
            println!("    (none)");
        } else {
            for (ext, count) in ext_list {
                println!("    .{}: {} files", ext, count);
            }
        }
    } else {
        // Compact view (top 5)
        let top_exts: Vec<_> = ext_list
            .iter()
            .take(5)
            .map(|(e, _)| format!(".{}", e))
            .collect();
        println!(
            "  {} {}",
            "Extensions:".dimmed(),
            if top_exts.is_empty() {
                "(none)".to_string()
            } else {
                top_exts.join(", ")
            }
        );
    }

    // Show files list if requested
    if show_files {
        println!();
        println!("{}", "📁 Indexed Files".cyan().bold());
        let mut sorted_files: Vec<_> = files.iter().collect();
        sorted_files.sort();
        for file in sorted_files.iter().take(50) {
            println!("  {}", file.dimmed());
        }
        if files.len() > 50 {
            println!("  {} ... and {} more", "".dimmed(), files.len() - 50);
        }
    }

    // Show helpful tip if graph is empty
    if result.nodes_extracted == 0 && result.files_indexed > 0 {
        println!();
        println!(
            "{} Files were scanned but no code nodes extracted.",
            "💡".yellow()
        );
        println!("   This may happen if files contain only comments or imports.");
    }

    Ok(())
}

/// Start the Agentic Bridge (MCP + Viz).
pub async fn bridge(path: &Path, launch_viz: bool, follow_symlinks: bool) -> Result<()> {
    use arbor_mcp::McpServer;

    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;

    eprintln!("{} Arbor Bridge (MCP Mode)", "🔗".bold().cyan());

    // 1. Create Shared Graph (Empty initially)
    let graph = arbor_graph::ArborGraph::new();
    let shared_graph = std::sync::Arc::new(tokio::sync::RwLock::new(graph));

    // 2. Run Initial Index (Blocking)
    let index_path = resolved_path.to_path_buf();
    let options = IndexOptions {
        follow_symlinks,
        cache_path: Some(resolved_path.join(".arbor").join("cache")),
    };
    eprintln!("{} Starting initial index...", "⏳".yellow());

    // Run blocking indexer
    let result = tokio::task::spawn_blocking(move || index_directory(&index_path, options)).await?;

    match result {
        Ok(index_result) => {
            let mut guard = shared_graph.write().await;
            *guard = index_result.graph;

            // Compute centrality
            let scores = compute_centrality(&guard, 20, 0.85);
            guard.set_centrality(scores.into_map());

            eprintln!(
                "{} Index Ready: {} files, {} nodes",
                "✓".green(),
                index_result.files_indexed,
                index_result.nodes_extracted
            );
        }
        Err(e) => eprintln!("{} Indexing failed: {}", "⚠".red(), e),
    }

    // Pass a clone to the background watcher/indexer (which we should start separately if we want continuous updates)
    // Actually, SyncServer handles the continuous watching!
    // The previous code had a separate background indexer that seemingly did nothing after the initial index?
    // No, wait. The previous code ONLY did the initial index.
    // The SyncServer (lines 355) is what handles *subsequent* file updates via its own watcher.
    // So this change is strictly correct.

    // 3. Start Servers (Background)
    let rpc_port = 7433;
    let sync_port = 8081;

    let rpc_config = ServerConfig {
        addr: format!("127.0.0.1:{}", rpc_port).parse()?,
    };

    let arbor_server = ArborServer::new_with_shared(shared_graph.clone(), rpc_config);

    let sync_config = arbor_server::SyncServerConfig {
        addr: format!("127.0.0.1:{}", sync_port).parse()?,
        watch_path: resolved_path.to_path_buf(),
        debounce_ms: 1000,
        extensions: vec![
            "rs".to_string(),
            "ts".to_string(),
            "tsx".to_string(),
            "js".to_string(),
            "jsx".to_string(),
            "py".to_string(),
            "dart".to_string(),
            "go".to_string(),
            "java".to_string(),
            "c".to_string(),
            "h".to_string(),
            "cpp".to_string(),
            "hpp".to_string(),
            "cc".to_string(),
            "cxx".to_string(),
            "hh".to_string(),
            "cs".to_string(),
            "kt".to_string(),
            "kts".to_string(),
            "swift".to_string(),
            "rb".to_string(),
            "php".to_string(),
            "phtml".to_string(),
            "sh".to_string(),
            "bash".to_string(),
            "zsh".to_string(),
        ],
    };

    let sync_server = arbor_server::SyncServer::new_with_shared(sync_config, shared_graph.clone());
    let spotlight_handle = sync_server.handle();

    tokio::spawn(async move {
        if let Err(e) = arbor_server.run().await {
            eprintln!("RPC Server error: {}", e);
        }
    });

    tokio::spawn(async move {
        if let Err(e) = sync_server.run().await {
            eprintln!("Sync Server error: {}", e);
        }
    });

    eprintln!(
        "{} Servers Ready (RPC {}, Sync {})",
        "✓".green(),
        rpc_port,
        sync_port
    );
    eprintln!("🔦 Spotlight mode active - Visualizer will track AI focus");

    // 3. Optionally launch the visualizer
    if launch_viz {
        // Try to find visualizer in target path or parent (workspace root)
        let viz_dir = if resolved_path.join("visualizer").exists() {
            Some(resolved_path.join("visualizer"))
        } else if Path::new("../visualizer").exists() {
            Some(Path::new("../visualizer").to_path_buf())
        } else {
            None
        };

        if let Some(dir) = viz_dir {
            eprintln!(
                "{} Launching Flutter Visualizer in {}...",
                "🚀".cyan(),
                dir.display()
            );

            #[cfg(target_os = "windows")]
            let (cmd, device) = ("flutter.bat", "windows");
            #[cfg(target_os = "macos")]
            let (cmd, device) = ("flutter", "macos");
            #[cfg(target_os = "linux")]
            let (cmd, device) = ("flutter", "linux");

            // Spawn visualizer in background
            std::process::Command::new(cmd)
                .arg("run")
                .arg("-d")
                .arg(device)
                .current_dir(&dir)
                .stdout(std::process::Stdio::null()) // Silence flutter output to keep MCP clean
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok();
        } else {
            eprintln!("{} Visualizer directory not found", "⚠".yellow());
        }
    }

    eprintln!("🚀 Starting MCP Server on Stdio... (Press Ctrl+C to stop)");

    // 3. Start MCP Server (Main Thread) WITH Spotlight capability
    // IMPORTANT: All logging MUST be to stderr from here on.
    let mcp = McpServer::with_spotlight(shared_graph, spotlight_handle);
    mcp.run_stdio().await?;

    Ok(())
}

/// Check system health and environment.
pub async fn check_health(path: Option<&Path>) -> Result<()> {
    use std::net::{TcpListener, TcpStream};

    println!("{}", "🔍 Arbor Health Check".cyan().bold());
    println!("{}", "═".repeat(50));

    let mut all_ok = true;

    // Detect workspace root (if we're in crates/, go up one level)
    let workspace_root = if let Some(input_path) = path {
        resolve_project_path(input_path)?
    } else {
        resolve_project_path(Path::new("."))?
    };

    println!(
        "{} Arbor version {}",
        "✓".green(),
        env!("CARGO_PKG_VERSION")
    );

    // 0. Check git repo
    if is_git_repo(&workspace_root) {
        println!("{} Git repository detected", "✓".green());
    } else {
        println!("{} Git repository not detected", "⚠".yellow());
        all_ok = false;
    }

    // 1. Check Cargo.toml presence (Rust workspace)
    let cargo_exists =
        Path::new("Cargo.toml").exists() || workspace_root.join("crates/Cargo.toml").exists();
    if cargo_exists {
        println!("{} Rust workspace detected", "✓".green());
    } else {
        println!(
            "{} No Cargo.toml found (not in a Rust project)",
            "⚠".yellow()
        );
    }

    // 2. Check port 8080 availability (SyncServer)
    match TcpListener::bind("127.0.0.1:8080") {
        Ok(_) => {
            println!("{} Port 8080 is available", "✓".green());
        }
        Err(_) => {
            println!(
                "{} Port 8080 is in use (SyncServer may be running)",
                "•".blue()
            );
        }
    }

    // 3. Check visualizer directory
    let viz_path = workspace_root.join("visualizer");
    if viz_path.exists() {
        println!("{} Visualizer directory found", "✓".green());
    } else {
        println!("{} Visualizer not found", "⚠".yellow());
    }

    // 4. Check VS Code extension
    let ext_path = workspace_root.join("extensions/arbor-vscode");
    if ext_path.exists() {
        println!("{} VS Code extension found", "✓".green());
    } else {
        println!("{} VS Code extension not found", "⚠".yellow());
    }

    // 5. Check .arbor directory
    let arbor_path = workspace_root.join(".arbor");
    if arbor_path.exists() {
        println!("{} Arbor initialized (.arbor/ exists)", "✓".green());

        // 6. Snapshot presence and size
        let snapshot = graph_snapshot_path(&workspace_root);
        if snapshot.exists() {
            let size = fs::metadata(&snapshot).map(|m| m.len()).unwrap_or(0);
            let size_mb = size as f64 / (1024.0 * 1024.0);
            if size_mb > 120.0 {
                println!(
                    "{} Graph snapshot is large ({:.1}MB) — consider prune/re-index",
                    "⚠".yellow(),
                    size_mb
                );
            } else {
                println!("{} Graph snapshot present ({:.1}MB)", "✓".green(), size_mb);
            }
        } else {
            println!("{} Graph snapshot not found", "⚠".yellow());
        }

        // 7. Cache/snapshot integrity
        match load_graph_binary(&workspace_root)
            .or_else(|_| load_graph_snapshot(&workspace_root))
            .or_else(|_| load_graph_from_store(&workspace_root))
        {
            Ok(_) => println!("{} Cache and snapshot readable", "✓".green()),
            Err(e) => {
                println!("{} Cache may be corrupted: {}", "⚠".yellow(), e);
                all_ok = false;
            }
        }

        // 8. Index freshness (git changes vs HEAD)
        match git_changed_files(&workspace_root) {
            Ok(changed) if changed.is_empty() => {
                println!(
                    "{} Index appears up to date (no pending git diffs)",
                    "✓".green()
                )
            }
            Ok(changed) => println!(
                "{} Index may be stale ({} changed files). Run 'arbor index --changed-only'",
                "⚠".yellow(),
                changed.len()
            ),
            Err(_) => println!("{} Could not determine index freshness", "⚠".yellow()),
        }
    } else {
        println!(
            "{} Arbor not initialized (run 'arbor setup' in workspace root)",
            "⚠".yellow()
        );
        all_ok = false;
    }

    // 9. MCP bridge health (best-effort)
    let mcp_healthy = TcpStream::connect("127.0.0.1:7433").is_ok();
    if mcp_healthy {
        println!(
            "{} MCP bridge appears healthy (port 7433 open)",
            "✓".green()
        );
    } else {
        println!(
            "{} MCP bridge not reachable on 7433 (start with 'arbor bridge')",
            "⚠".yellow()
        );
    }

    println!("{}", "═".repeat(50));

    if all_ok {
        println!("{} All systems operational", "🚀".green().bold());
    } else {
        println!("{}", "⚠  Some checks require attention".yellow());
    }

    Ok(())
}

pub fn refactor(
    target: &str,
    max_depth: usize,
    show_why: bool,
    json_output: bool,
    path: &Path,
) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    // Find the target node
    let node_idx = graph.get_index(target).or_else(|| {
        graph
            .find_by_name(target)
            .first()
            .and_then(|n| graph.get_index(&n.id))
    });

    let node_idx = match node_idx {
        Some(idx) => idx,
        None => {
            // Smart fallback: suggest similar symbols
            return suggest_similar_symbols(&graph, target);
        }
    };

    // Get the target node info
    let target_node = graph.get(node_idx).unwrap();

    // Run impact analysis
    let analysis = graph.analyze_impact(node_idx, max_depth);

    if json_output {
        // JSON output (keep existing behavior for automation)
        let output = serde_json::json!({
            "target": {
                "id": analysis.target.id,
                "name": analysis.target.name,
                "kind": analysis.target.kind,
                "file": analysis.target.file
            },
            "upstream": analysis.upstream.iter().map(|n| serde_json::json!({
                "id": n.node_info.id,
                "name": n.node_info.name,
                "severity": n.severity.as_str(),
                "hop_distance": n.hop_distance,
                "entry_edge": n.entry_edge.to_string()
            })).collect::<Vec<_>>(),
            "downstream": analysis.downstream.iter().map(|n| serde_json::json!({
                "id": n.node_info.id,
                "name": n.node_info.name,
                "severity": n.severity.as_str(),
                "hop_distance": n.hop_distance,
                "entry_edge": n.entry_edge.to_string()
            })).collect::<Vec<_>>(),
            "total_affected": analysis.total_affected,
            "query_time_ms": analysis.query_time_ms
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    // === WARM, OPINIONATED OUTPUT ===
    println!();
    println!(
        "{} {}",
        "🔍 Analyzing".cyan().bold(),
        target_node.name.cyan().bold()
    );
    println!();

    // Compute and display confidence
    let confidence = arbor_graph::ConfidenceExplanation::from_analysis(&analysis);
    let role = arbor_graph::NodeRole::from_analysis(&analysis);

    let confidence_color = match confidence.level {
        arbor_graph::ConfidenceLevel::High => "green",
        arbor_graph::ConfidenceLevel::Medium => "yellow",
        arbor_graph::ConfidenceLevel::Low => "red",
    };

    println!(
        "{}  {} | {}",
        match confidence.level {
            arbor_graph::ConfidenceLevel::High => "🟢",
            arbor_graph::ConfidenceLevel::Medium => "🟡",
            arbor_graph::ConfidenceLevel::Low => "🔴",
        },
        format!("Confidence: {}", confidence.level).color(confidence_color),
        format!("Role: {}", role).dimmed()
    );

    for reason in &confidence.reasons {
        println!("   • {}", reason.dimmed());
    }
    println!();

    // ========== --why VERBOSE OUTPUT ==========
    if show_why {
        println!("{}", "═══ Detailed Analysis (--why) ═══".cyan().bold());
        println!();

        // 1. Why this confidence level?
        println!("{}", "📊 Why this confidence level?".cyan());
        match confidence.level {
            arbor_graph::ConfidenceLevel::High => {
                println!("   • High caller count indicates well-integrated code");
                println!("   • Clear static call graph with minimal uncertainty");
            }
            arbor_graph::ConfidenceLevel::Medium => {
                println!("   • Moderate caller count or some uncertain edges");
                println!("   • May have dynamic dispatch or callback patterns");
            }
            arbor_graph::ConfidenceLevel::Low => {
                println!("   • Few or no callers detected statically");
                println!("   • May be called via reflection, DI, or externally");
            }
        }
        println!();

        // 2. Check for heuristics fired
        let _all_nodes: Vec<_> = analysis
            .all_affected()
            .iter()
            .map(|a| &a.node_info)
            .collect();
        let all_node_refs: Vec<_> = graph.nodes().take(100).collect(); // Sample for heuristics

        let callbacks: Vec<_> = all_node_refs
            .iter()
            .filter(|n| arbor_graph::HeuristicsMatcher::is_callback_style(n))
            .take(3)
            .collect();
        let event_handlers: Vec<_> = all_node_refs
            .iter()
            .filter(|n| arbor_graph::HeuristicsMatcher::is_event_handler(n))
            .take(3)
            .collect();
        let widgets: Vec<_> = all_node_refs
            .iter()
            .filter(|n| arbor_graph::HeuristicsMatcher::is_flutter_widget(n))
            .take(3)
            .collect();
        let di_nodes: Vec<_> = all_node_refs
            .iter()
            .filter(|n| arbor_graph::HeuristicsMatcher::is_dependency_injection(n))
            .take(3)
            .collect();

        println!("{}", "🔍 Heuristics detected in codebase:".cyan());
        if callbacks.is_empty()
            && event_handlers.is_empty()
            && widgets.is_empty()
            && di_nodes.is_empty()
        {
            println!("   • None detected (clean static analysis)");
        } else {
            if !callbacks.is_empty() {
                println!(
                    "   • {} callback-style nodes (may be invoked dynamically)",
                    callbacks.len()
                );
                for cb in &callbacks {
                    println!("     └─ {}", cb.name.dimmed());
                }
            }
            if !event_handlers.is_empty() {
                println!(
                    "   • {} event handlers (connected at runtime)",
                    event_handlers.len()
                );
                for eh in &event_handlers {
                    println!("     └─ {}", eh.name.dimmed());
                }
            }
            if !widgets.is_empty() {
                println!(
                    "   • {} Flutter widgets (tree determined at runtime)",
                    widgets.len()
                );
            }
            if !di_nodes.is_empty() {
                println!(
                    "   • {} DI/factory patterns (may bypass static calls)",
                    di_nodes.len()
                );
            }
        }
        println!();

        // 3. Why were callers included/excluded?
        println!("{}", "📥 Why callers were included:".cyan());
        if analysis.upstream.is_empty() {
            println!("   • No static callers found in indexed files");
            println!("   • Check: external entry points, tests, or dynamic invocation");
        } else {
            println!(
                "   • {} nodes call this directly or transitively",
                analysis.upstream.len()
            );
            for caller in analysis.upstream.iter().take(3) {
                println!(
                    "     └─ {} via {}",
                    caller.node_info.name,
                    caller.entry_edge.to_string().dimmed()
                );
            }
        }
        println!();

        println!("{}", "📤 Why dependencies were included:".cyan());
        if analysis.downstream.is_empty() {
            println!("   • This is a leaf node (no outgoing calls)");
        } else {
            println!(
                "   • {} nodes are called by this function",
                analysis.downstream.len()
            );
        }
        println!();

        println!("{}", "════════════════════════════════".dimmed());
        println!();
    }

    // Determine the node's role
    let has_upstream = !analysis.upstream.is_empty();
    let has_downstream = !analysis.downstream.is_empty();

    match (has_upstream, has_downstream) {
        (false, false) => {
            // Isolated node
            println!("{}", "This node appears isolated.".yellow());
            println!("  • No callers found in the codebase");
            println!("  • No dependencies detected");
            println!();
            println!("{}", "Possible reasons:".dimmed());
            println!("  • It's an entry point called externally (CLI, HTTP, tests)");
            println!("  • It's dynamically invoked (reflection, callbacks)");
            println!("  • It may be dead code");
            println!();
            println!("{} Safe to change, but verify external usage.", "→".green());
        }
        (false, true) => {
            // Entry point (no callers, but calls others)
            println!("{}", "This is an entry point.".green());
            println!("  Nothing in your codebase calls it directly.");
            println!();
            println!("{}", "However, changing it may affect:".yellow());
            for node in analysis.downstream.iter().take(5) {
                println!(
                    "  └─ {} ({})",
                    node.node_info.name.cyan(),
                    node.entry_edge.to_string().dimmed()
                );
            }
            if analysis.downstream.len() > 5 {
                println!("  └─ ... and {} more", analysis.downstream.len() - 5);
            }
            println!();
            println!(
                "{} Low risk upstream, {} downstream dependencies.",
                "→".green(),
                analysis.downstream.len().to_string().yellow()
            );
        }
        (true, false) => {
            // Leaf/utility node (has callers, but doesn't call anything)
            println!("{}", "This is a utility function.".cyan());
            println!("  Called by others, but doesn't depend on much.");
            println!();
            println!("{}", "Called by:".yellow());
            for node in analysis.upstream.iter().take(5) {
                println!(
                    "  • {} ({} hop{})",
                    node.node_info.name.cyan(),
                    node.hop_distance,
                    if node.hop_distance == 1 { "" } else { "s" }
                );
            }
            if analysis.upstream.len() > 5 {
                println!("  • ... and {} more", analysis.upstream.len() - 5);
            }
            println!();
            println!(
                "{} Changes here ripple up to {} caller{}.",
                "→".yellow(),
                analysis.upstream.len(),
                if analysis.upstream.len() == 1 {
                    ""
                } else {
                    "s"
                }
            );
        }
        (true, true) => {
            // Connected node (has both callers and dependencies)
            println!("{}", "This node sits in the middle of the graph.".cyan());
            println!(
                "  {} caller{}, {} dependenc{}.",
                analysis.upstream.len(),
                if analysis.upstream.len() == 1 {
                    ""
                } else {
                    "s"
                },
                analysis.downstream.len(),
                if analysis.downstream.len() == 1 {
                    "y"
                } else {
                    "ies"
                }
            );
            println!();

            // Count by severity
            let direct: Vec<_> = analysis
                .all_affected()
                .into_iter()
                .filter(|n| n.severity == arbor_graph::ImpactSeverity::Direct)
                .collect();
            let transitive: Vec<_> = analysis
                .all_affected()
                .into_iter()
                .filter(|n| n.severity == arbor_graph::ImpactSeverity::Transitive)
                .collect();

            println!(
                "{} {} nodes affected ({}  direct, {} transitive)",
                "⚠️ ".yellow(),
                analysis.total_affected.to_string().bold(),
                direct.len().to_string().red(),
                transitive.len().to_string().yellow()
            );
            println!();

            if !direct.is_empty() {
                println!("{}", "Will break immediately:".red());
                for node in direct.iter().take(5) {
                    print!("  • {} ({})", node.node_info.name, node.node_info.kind);
                    if show_why {
                        print!(
                            " — {} {}",
                            node.entry_edge.to_string().dimmed(),
                            target_node.name
                        );
                    }
                    println!();
                }
                if direct.len() > 5 {
                    println!("  • ... and {} more", direct.len() - 5);
                }
                println!();
            }

            if !transitive.is_empty() && show_why {
                println!("{}", "May break indirectly:".yellow());
                for node in transitive.iter().take(3) {
                    println!(
                        "  • {} ({} hops away)",
                        node.node_info.name, node.hop_distance
                    );
                }
                if transitive.len() > 3 {
                    println!("  • ... and {} more", transitive.len() - 3);
                }
                println!();
            }

            println!("{} Proceed carefully. Test affected callers.", "→".red());
        }
    }

    println!();
    println!("{}", format!("File: {}", target_node.file).dimmed());

    Ok(())
}

/// Suggest similar symbols when exact match fails
fn suggest_similar_symbols(graph: &arbor_graph::ArborGraph, target: &str) -> Result<()> {
    println!();
    println!("{} Couldn't find \"{}\"", "🔍".yellow(), target.cyan());
    println!();

    // Find symbols with relevance scoring
    let target_lower = target.to_lowercase();

    // (node, relevance_score, caller_count)
    // Relevance: 100 = exact name, 80 = exact suffix, 60 = starts with, 40 = contains, 30 = fuzzy
    let mut suggestions: Vec<(&arbor_core::CodeNode, u32, usize)> = Vec::new();

    for node in graph.nodes() {
        let name_lower = node.name.to_lowercase();
        let id_lower = node.id.to_lowercase();

        let relevance = if name_lower == target_lower {
            100 // Exact name match
        } else if id_lower.ends_with(&format!("::{}", target_lower))
            || id_lower.ends_with(&format!(".{}", target_lower))
        {
            80 // Exact suffix match (e.g., "auth" matches "module::auth")
        } else if name_lower.starts_with(&target_lower) {
            60 // Starts with (e.g., "auth" matches "authenticate")
        } else if name_lower.contains(&target_lower) {
            40 // Contains (e.g., "auth" matches "user_auth_handler")
        } else {
            // Fuzzy matching using Jaro-Winkler similarity (good for typos)
            let similarity = strsim::jaro_winkler(&name_lower, &target_lower);
            if similarity > 0.75 {
                30 // Fuzzy match (e.g., "autth" → "auth")
            } else {
                continue; // No match
            }
        };

        // Count callers for this node
        let caller_count = if let Some(idx) = graph.get_index(&node.id) {
            graph.analyze_impact(idx, 1).upstream.len()
        } else {
            0
        };
        suggestions.push((node, relevance, caller_count));
    }

    // Sort by relevance first, then by caller count
    suggestions.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.2.cmp(&a.2)));

    if suggestions.is_empty() {
        println!("No similar symbols found in the codebase.");
        println!();
        println!("{}", "Tips:".dimmed());
        println!("  • Check spelling");
        println!("  • Use the full qualified name (e.g., module::function)");
        println!("  • Run `arbor query <name>` to search");
        return Ok(());
    }

    println!("{}", "Did you mean:".green());
    for (i, (node, _relevance, caller_count)) in suggestions.iter().take(3).enumerate() {
        let suffix = if *caller_count == 0 {
            "entry point".dimmed().to_string()
        } else {
            format!(
                "{} caller{}",
                caller_count,
                if *caller_count == 1 { "" } else { "s" }
            )
        };
        println!("  {}) {} — {}", i + 1, node.id.cyan(), suffix);
    }

    if !suggestions.is_empty() {
        println!();
        println!(
            "Run: {}",
            format!("arbor refactor {}", suggestions[0].0.id).green()
        );
    }

    Ok(())
}

pub fn explain(
    question: &str,
    max_tokens: usize,
    show_why: bool,
    json_output: bool,
    path: &Path,
) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    // Try to find a node matching the question (could be a function name)
    let node_idx = graph.get_index(question).or_else(|| {
        graph
            .find_by_name(question)
            .first()
            .and_then(|n| graph.get_index(&n.id))
    });

    let node_idx = match node_idx {
        Some(idx) => idx,
        None => {
            return Err(format!("Node '{}' not found in graph", question).into());
        }
    };

    // Slice context around the node
    let slice = graph.slice_context(node_idx, max_tokens, 2, &[]);

    // Warn if context was truncated
    if slice.truncation_reason != arbor_graph::TruncationReason::Complete {
        eprintln!(
            "\n{} Context truncated: {} (limit: {} tokens)",
            "⚠".yellow(),
            slice.truncation_reason,
            max_tokens
        );
        eprintln!("  Some nodes were excluded to fit token budget.");
        eprintln!("  Use --tokens to increase limit, or use pinning for critical nodes.");
    }

    if json_output {
        let output = serde_json::json!({
            "target": {
                "id": slice.target.id,
                "name": slice.target.name,
                "kind": slice.target.kind,
                "file": slice.target.file
            },
            "context_nodes": slice.nodes.iter().map(|n| serde_json::json!({
                "id": n.node_info.id,
                "name": n.node_info.name,
                "kind": n.node_info.kind,
                "file": n.node_info.file,
                "depth": n.depth,
                "token_estimate": n.token_estimate,
                "pinned": n.pinned
            })).collect::<Vec<_>>(),
            "total_tokens": slice.total_tokens,
            "max_tokens": slice.max_tokens,
            "truncation_reason": slice.truncation_reason.to_string()
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}", "📖 Graph-Backed Context".cyan().bold());
        println!(
            "Target: {} ({})",
            slice.target.name.cyan(),
            slice.target.kind
        );
        println!();

        println!("{}", slice.summary());
        println!();

        if show_why {
            println!("{}", "Path traced:".dimmed());
            for node in slice.nodes.iter().take(10) {
                let pinned_marker = if node.pinned { " [pinned]" } else { "" };
                println!(
                    "  {} {} ({}) — ~{} tokens{}",
                    "→".dimmed(),
                    node.node_info.name,
                    node.node_info.kind,
                    node.token_estimate,
                    pinned_marker.cyan()
                );
            }
            if slice.nodes.len() > 10 {
                println!("  ... and {} more nodes", slice.nodes.len() - 10);
            }
            println!();
        }

        println!(
            "Truncation: {} | Query time: {}ms",
            slice.truncation_reason.to_string().yellow(),
            slice.query_time_ms
        );
    }

    Ok(())
}

pub fn open(symbol: &str, path: &Path) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    let (file, line) = resolve_node_or_file_target(&graph, symbol, &resolved_path)
        .ok_or_else(|| format!("Could not resolve symbol or file '{}'.", symbol))?;

    open_in_editor(&file, line)?;
    println!("{} Opened {}:{}", "✓".green(), file, line);
    Ok(())
}

/// Launch the graphical interface.
pub fn gui(path: &Path) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;
    println!("{} Launching Arbor GUI...", "🌲".green());

    // Set the working directory for the GUI
    std::env::set_current_dir(&resolved_path)?;

    // Find the arbor-gui executable
    let exe_dir = std::env::current_exe()?.parent().unwrap().to_path_buf();

    #[cfg(target_os = "windows")]
    let gui_exe = exe_dir.join("arbor-gui.exe");
    #[cfg(not(target_os = "windows"))]
    let gui_exe = exe_dir.join("arbor-gui");

    if gui_exe.exists() {
        // Launch the GUI executable
        std::process::Command::new(&gui_exe)
            .spawn()
            .map_err(|e| format!("Failed to launch GUI: {}", e))?;
        println!("  GUI started. Analyzing: {}", path.display());
    } else {
        // Try cargo run as fallback for development
        println!(
            "  {} GUI executable not found at {:?}",
            "⚠".yellow(),
            gui_exe
        );
        println!("  Running in development mode...");
        std::process::Command::new("cargo")
            .args(["run", "--package", "arbor-gui"])
            .current_dir(&resolved_path)
            .spawn()
            .map_err(|e| format!("Failed to launch GUI: {}", e))?;
    }

    Ok(())
}

/// Generate a PR summary for refactored symbols.
pub fn pr_summary(symbols: &str, path: &Path) -> Result<()> {
    println!("{}", "📝 PR Summary Generator".cyan().bold());
    println!();

    // Index the codebase
    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;
    let graph = load_or_index_graph(&resolved_path)?;

    let symbol_list: Vec<&str> = symbols.split(',').map(|s| s.trim()).collect();

    println!("## Impact Analysis\n");
    println!("The following symbols were modified:\n");

    for symbol in &symbol_list {
        // Find the node
        let node_idx = graph.get_index(symbol).or_else(|| {
            graph
                .find_by_name(symbol)
                .first()
                .and_then(|n| graph.get_index(&n.id))
        });

        if let Some(idx) = node_idx {
            let node = graph.get(idx).unwrap();
            let analysis = graph.analyze_impact(idx, 3);
            let confidence = arbor_graph::ConfidenceExplanation::from_analysis(&analysis);
            let role = arbor_graph::NodeRole::from_analysis(&analysis);

            println!("### `{}`", node.name);
            println!();
            println!("- **File:** `{}`", node.file);
            println!("- **Role:** {}", role);
            println!("- **Confidence:** {}", confidence.level);
            println!("- **Total Affected:** {} nodes", analysis.total_affected);

            if !analysis.upstream.is_empty() {
                println!("\n**Callers that may be affected:**");
                for caller in analysis.upstream.iter().take(5) {
                    println!("- `{}`", caller.node_info.name);
                }
            }
            println!();
        } else {
            println!("### `{}` (not found in graph)\n", symbol);
        }
    }

    println!("---");
    println!("*Generated by Arbor*");

    Ok(())
}

/// Watch for file changes and re-index automatically.
pub async fn watch(path: &Path) -> Result<()> {
    use std::time::Duration;

    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;

    println!("{}", "👁️  Watch Mode".cyan().bold());
    println!("Watching: {}", resolved_path.display());
    println!("Press Ctrl+C to stop.\n");

    // Initial index
    let mut last_result = index_directory(&resolved_path, IndexOptions::default())?;
    println!(
        "✓ Initial index: {} files, {} nodes",
        last_result.files_indexed, last_result.nodes_extracted
    );

    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Re-index and check for changes
        match index_directory(&resolved_path, IndexOptions::default()) {
            Ok(result) => {
                if result.nodes_extracted != last_result.nodes_extracted
                    || result.files_indexed != last_result.files_indexed
                {
                    println!(
                        "🔄 Updated: {} files, {} nodes (was {} files, {} nodes)",
                        result.files_indexed,
                        result.nodes_extracted,
                        last_result.files_indexed,
                        last_result.nodes_extracted
                    );
                    last_result = result;
                }
            }
            Err(e) => {
                eprintln!("⚠ Index error: {}", e);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{is_generated_or_internal_path, parse_git_name_status_output};
    use std::path::PathBuf;

    /// Returns the platform-specific bundled visualizer path relative to exe_dir.
    fn get_bundled_visualizer_path(exe_dir: &std::path::Path) -> PathBuf {
        #[cfg(target_os = "windows")]
        {
            exe_dir.join("arbor_visualizer").join("visualizer.exe")
        }
        #[cfg(target_os = "macos")]
        {
            exe_dir
                .join("arbor_visualizer")
                .join("arbor_visualizer.app")
                .join("Contents")
                .join("MacOS")
                .join("arbor_visualizer")
        }
        #[cfg(target_os = "linux")]
        {
            exe_dir.join("arbor_visualizer").join("arbor_visualizer")
        }
    }

    /// Returns the platform-specific Flutter command and device target.
    fn get_flutter_cmd_and_device() -> (&'static str, &'static str) {
        #[cfg(target_os = "windows")]
        {
            ("flutter.bat", "windows")
        }
        #[cfg(target_os = "macos")]
        {
            ("flutter", "macos")
        }
        #[cfg(target_os = "linux")]
        {
            ("flutter", "linux")
        }
    }

    #[test]
    fn test_bundled_visualizer_path_structure() {
        let exe_dir = PathBuf::from("/usr/local/bin");
        let viz_path = get_bundled_visualizer_path(&exe_dir);

        #[cfg(target_os = "windows")]
        assert!(viz_path.to_string_lossy().ends_with("visualizer.exe"));

        #[cfg(target_os = "macos")]
        {
            assert!(viz_path.to_string_lossy().contains("arbor_visualizer.app"));
            assert!(viz_path.to_string_lossy().contains("Contents/MacOS"));
        }

        #[cfg(target_os = "linux")]
        {
            assert!(viz_path.to_string_lossy().ends_with("arbor_visualizer"));
            assert!(!viz_path.to_string_lossy().contains(".exe"));
            assert!(!viz_path.to_string_lossy().contains(".app"));
        }
    }

    #[test]
    fn test_flutter_device_target() {
        let (cmd, device) = get_flutter_cmd_and_device();

        #[cfg(target_os = "windows")]
        {
            assert_eq!(cmd, "flutter.bat");
            assert_eq!(device, "windows");
        }

        #[cfg(target_os = "macos")]
        {
            assert_eq!(cmd, "flutter");
            assert_eq!(device, "macos");
        }

        #[cfg(target_os = "linux")]
        {
            assert_eq!(cmd, "flutter");
            assert_eq!(device, "linux");
        }
    }

    #[test]
    fn test_bundled_visualizer_path_is_absolute_when_exe_dir_is_absolute() {
        #[cfg(target_os = "windows")]
        let exe_dir = PathBuf::from("C:\\Program Files\\Arbor\\bin");
        #[cfg(not(target_os = "windows"))]
        let exe_dir = PathBuf::from("/opt/arbor/bin");

        let viz_path = get_bundled_visualizer_path(&exe_dir);
        assert!(
            viz_path.is_absolute(),
            "Expected absolute path, got: {:?}",
            viz_path
        );
    }

    #[test]
    fn test_parse_git_name_status_output_handles_rename_modify_delete() {
        let output = "R100\tsrc/old.rs\tsrc/new.rs\nM\tsrc/lib.rs\nD\tsrc/dead.rs\n";
        let parsed = parse_git_name_status_output(output);

        assert!(parsed.contains(&"src/new.rs".to_string()));
        assert!(parsed.contains(&"src/lib.rs".to_string()));
        assert!(!parsed.contains(&"src/old.rs".to_string()));
        assert!(!parsed.contains(&"src/dead.rs".to_string()));
    }

    #[test]
    fn test_generated_or_internal_path_filter() {
        assert!(is_generated_or_internal_path(".arbor/config.json"));
        assert!(is_generated_or_internal_path("target/debug/foo"));
        assert!(is_generated_or_internal_path("src/models/user.g.dart"));
        assert!(is_generated_or_internal_path("pkg/generated/client.rs"));
        assert!(!is_generated_or_internal_path("src/lib.rs"));
    }
}

/// Perform a security audit to find paths to a sensitive sink.
pub fn audit(sink: &str, depth: usize, format: &str, path: &Path) -> Result<()> {
    let resolved_path = resolve_project_path(path)?;

    // 1. Load the graph
    let graph = load_or_index_graph(&resolved_path)?;
    println!(
        "{} Auditing security paths to sink: {}",
        "🔍".cyan(),
        sink.yellow().bold()
    );

    // 2. Configure audit
    let config = crate::audit::AuditConfig {
        max_depth: depth,
        ignore_tests: true,
    };

    // 3. Run audit
    let start = std::time::Instant::now();
    let result = crate::audit::run_audit(&graph, sink, &config).map_err(|e| e.to_string())?;
    let duration = start.elapsed();

    // 4. Output results
    match format {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&result)?);
            return Ok(());
        }
        "csv" => {
            println!("severity,entry_point,entry_file,path_length,trace");
            for audit_path in &result.paths {
                let trace_str: Vec<&str> =
                    audit_path.trace.iter().map(|n| n.name.as_str()).collect();
                println!(
                    "{},{},{},{},\"{}\"",
                    audit_path.severity.label(),
                    audit_path.source.name,
                    audit_path.source.file,
                    audit_path.trace.len(),
                    trace_str.join(" -> ")
                );
            }
            return Ok(());
        }
        _ => {} // text format below
    }

    // Text output
    println!(
        "\n{} Found {} paths to sink in {:.2?}",
        if result.path_count > 0 {
            "⚠️".yellow()
        } else {
            "✓".green()
        },
        result.path_count,
        duration
    );

    if result.path_count == 0 {
        println!(
            "\nNo public entry points found leading to '{}'.",
            sink.dimmed()
        );
        return Ok(());
    }

    // Summary box
    println!("\n{}", "┌─ Audit Summary ─────────────────────┐".dimmed());
    println!(
        "│  🔴 Critical: {}  🟠 High: {}  🟡 Medium: {}  🟢 Low: {}",
        result.summary.critical_count,
        result.summary.high_count,
        result.summary.medium_count,
        result.summary.low_count
    );
    println!(
        "│  Entry Points: {}  Files Touched: {}",
        result.summary.unique_entry_points, result.summary.unique_files
    );
    println!("{}", "└─────────────────────────────────────┘".dimmed());

    // Detailed paths
    println!("\n{}", "Exploit Paths:".red().bold());
    println!("{}", "═".repeat(50).dimmed());

    for (i, audit_path) in result.paths.iter().take(15).enumerate() {
        println!(
            "\n{} {}. {} → {}",
            audit_path.severity.emoji(),
            i + 1,
            audit_path.source.name.green().bold(),
            sink.red().bold()
        );
        println!(
            "   {} {}  Depth: {}",
            "File:".dimmed(),
            audit_path.source.file.dimmed(),
            audit_path.trace.len()
        );

        println!("   {}", "Trace:".dimmed());
        for (j, step) in audit_path.trace.iter().enumerate() {
            let is_last = j == audit_path.trace.len() - 1;
            let prefix = if is_last { "└─" } else { "├─" };
            let name = if j == 0 {
                step.name.green().to_string()
            } else if is_last {
                step.name.red().to_string()
            } else {
                step.name.white().to_string()
            };
            println!("     {} {}", prefix.dimmed(), name);
        }

        if !audit_path.uncertainty.is_empty() {
            println!(
                "   {} {}",
                "⚠ Heuristic:".yellow(),
                audit_path.uncertainty.join(", ")
            );
        }
    }

    if result.path_count > 15 {
        println!(
            "\n{} ... and {} more paths. Use --format json for full export.",
            "→".dimmed(),
            result.path_count - 15
        );
    }

    // Remediation
    println!("\n{}", "Recommended Actions:".cyan().bold());
    println!(
        "  1. Review direct callers of '{}' for input validation.",
        sink
    );
    println!("  2. Add sanitization at entry points marked CRITICAL/HIGH.");
    println!(
        "  3. Export full report: {} {} --format csv",
        "arbor audit".bold(),
        sink
    );

    Ok(())
}
