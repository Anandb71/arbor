//! CLI command implementations.

use arbor_mcp::list_changed_files;
use arbor_watcher::{index_directory, IndexOptions};
use colored::Colorize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

mod agent;
mod analyze;
mod impact;
mod index;
mod query;
mod serve;

pub use agent::{agent_guard, agent_onboard, agent_review};
pub use analyze::{audit, explain, pr_summary, refactor, summary};
pub use impact::{check, diff};
pub use index::{export, index, init, status};
pub use query::{
    callees, callers, entry_points, file_graph, find_path_cmd, inspect, map, open, query,
};
pub use serve::{bridge, check_health, gui, serve, viz, watch};

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

/// Removes Windows' extended-length path prefix.
///
/// `fs::canonicalize` returns verbatim paths (`\\?\C:\...`) on Windows. That
/// prefix flowed into every stored node path and therefore into every line of
/// user-facing output, where it is noise at best and confusing at worst.
pub(crate) fn strip_verbatim_prefix(path: PathBuf) -> PathBuf {
    match path.to_str() {
        Some(s) => {
            if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
                PathBuf::from(format!(r"\\{rest}"))
            } else if let Some(rest) = s.strip_prefix(r"\\?\") {
                PathBuf::from(rest)
            } else {
                path
            }
        }
        None => path,
    }
}

pub(crate) fn find_workspace_root(start: &Path) -> PathBuf {
    let mut current =
        strip_verbatim_prefix(fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf()));
    if current.is_file() {
        if let Some(parent) = current.parent() {
            current = parent.to_path_buf();
        }
    }

    let fallback = current.clone();
    loop {
        if ROOT_MARKERS
            .iter()
            .any(|marker| current.join(marker).exists())
        {
            return current;
        }

        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }

    fallback
}

pub(crate) fn resolve_project_path(path: &Path) -> Result<PathBuf> {
    let base = if path == Path::new(".") {
        std::env::current_dir()?
    } else {
        path.to_path_buf()
    };
    Ok(find_workspace_root(&base))
}

/// Whether Arbor may index a project that has no `.arbor/` directory yet.
///
/// Off by default so commands run against an un-indexed project (e.g. a
/// different repo) don't silently create a `.arbor/` and write a cache into it.
/// Resolution order: `ARBOR_AUTO_INDEX` env var, then `auto_index` in the
/// global config (`~/.arbor/config.json`), then `false`.
pub(crate) fn auto_index_enabled() -> bool {
    if let Ok(val) = std::env::var("ARBOR_AUTO_INDEX") {
        return matches!(
            val.trim().to_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        );
    }
    global_config_auto_index().unwrap_or(false)
}

/// Reads `auto_index` from the global config at `~/.arbor/config.json`.
pub(crate) fn global_config_auto_index() -> Option<bool> {
    let config_path = dirs::home_dir()?.join(".arbor").join("config.json");
    let text = fs::read_to_string(config_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get("auto_index").and_then(|v| v.as_bool())
}

/// Returns true if `path` has already been indexed (has a `.arbor/` directory).
pub(crate) fn project_is_indexed(path: &Path) -> bool {
    path.join(".arbor").exists()
}

/// Error returned when a command needs an indexed project but the project is
/// un-indexed and auto-indexing is disabled.
pub(crate) fn not_indexed_error(path: &Path) -> Box<dyn std::error::Error> {
    format!(
        "Project at {} is not indexed and auto-indexing is disabled.\n  \
         Run 'arbor index {}' to index it, or enable auto-indexing with \
         ARBOR_AUTO_INDEX=1 (or \"auto_index\": true in ~/.arbor/config.json).",
        path.display(),
        path.display()
    )
    .into()
}

/// Ensures `.arbor/` exists for an *implicit* (non-`index`/`init`) command.
///
/// If the project is already indexed, this is a no-op create. If it is NOT
/// indexed, it only creates `.arbor/` when auto-indexing is enabled; otherwise
/// it errors rather than mutating the project.
pub(crate) fn ensure_arbor_initialized(path: &Path) -> Result<bool> {
    if !project_is_indexed(path) && !auto_index_enabled() {
        return Err(not_indexed_error(path));
    }
    init_arbor_dir(path)
}

/// Creates and populates `.arbor/` unconditionally. Used by the explicit
/// `init`/`index`/`setup` commands, which always opt the user into indexing.
pub(crate) fn init_arbor_dir(path: &Path) -> Result<bool> {
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

pub(crate) fn graph_snapshot_path(path: &Path) -> PathBuf {
    path.join(".arbor").join("graph.json")
}

pub(crate) fn graph_binary_path(path: &Path) -> PathBuf {
    path.join(".arbor").join("graph.bin")
}

pub(crate) fn graph_store_path(path: &Path) -> PathBuf {
    path.join(".arbor").join("cache")
}

pub(crate) fn save_graph_snapshot(path: &Path, graph: &arbor_graph::ArborGraph) -> Result<()> {
    let graph_path = graph_snapshot_path(path);
    if let Some(parent) = graph_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = graph_path.with_extension("json.tmp");
    let file = std::fs::File::create(&tmp_path)?;
    let writer = std::io::BufWriter::new(file);
    serde_json::to_writer_pretty(writer, graph)?;
    if let Err(e) = fs::rename(&tmp_path, &graph_path) {
        // `rename` may fail to overwrite an existing destination on some platforms (e.g. Windows).
        if graph_path.exists() {
            fs::remove_file(&graph_path)?;
            fs::rename(&tmp_path, &graph_path)?;
        } else {
            return Err(e.into());
        }
    }
    Ok(())
}

pub(crate) fn save_graph_binary(path: &Path, graph: &arbor_graph::ArborGraph) -> Result<()> {
    let graph_path = graph_binary_path(path);
    if let Some(parent) = graph_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = graph_path.with_extension("bin.tmp");
    let bytes = bincode::serialize(graph)?;
    fs::write(&tmp_path, bytes)?;
    fs::rename(&tmp_path, &graph_path)?;
    Ok(())
}

pub(crate) fn load_graph_snapshot(path: &Path) -> Result<arbor_graph::ArborGraph> {
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
    let mut graph: arbor_graph::ArborGraph = serde_json::from_reader(reader)?;
    graph.rebuild_search_index();
    Ok(graph)
}

pub(crate) fn load_graph_binary(path: &Path) -> Result<arbor_graph::ArborGraph> {
    let graph_path = graph_binary_path(path);
    if !graph_path.exists() {
        return Err(format!("Binary graph not found at {}", graph_path.display()).into());
    }

    let bytes = fs::read(graph_path)?;
    let mut graph: arbor_graph::ArborGraph = bincode::deserialize(&bytes)?;
    graph.rebuild_search_index();
    Ok(graph)
}

pub(crate) fn load_graph_from_store(path: &Path) -> Result<arbor_graph::ArborGraph> {
    let store_path = graph_store_path(path);
    if !store_path.exists() {
        return Err("No graph store cache found".into());
    }

    let store = arbor_graph::GraphStore::open_or_reset(&store_path)
        .map_err(|e| format!("Failed to open graph store: {}", e))?;

    let mut graph = store
        .load_graph()
        .map_err(|e| format!("Failed to load graph from store: {}", e))?;

    if graph.node_count() == 0 {
        return Err("Graph store was empty".into());
    }

    graph.rebuild_search_index();
    Ok(graph)
}

/// Returns the modified time of a cache file in seconds since the UNIX epoch.
pub(crate) fn cache_mtime_secs(cache_path: &Path) -> Option<u64> {
    fs::metadata(cache_path)
        .and_then(|m| m.modified())
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Returns true if a source file is newer than the freshest cache file,
/// meaning the cached graph is stale and should be rebuilt from source.
pub(crate) fn cache_is_stale(path: &Path) -> bool {
    // Compare against the freshest of the two cache files — a running bridge's
    // periodic writer keeps graph.bin current, so prefer whichever is newer.
    let newest = [graph_binary_path(path), graph_snapshot_path(path)]
        .iter()
        .filter_map(|p| cache_mtime_secs(p))
        .max();
    match newest {
        Some(cache_mtime) => arbor_watcher::sources_newer_than(path, cache_mtime, false),
        None => false, // no cache yet; nothing to call stale
    }
}

pub(crate) fn load_or_index_graph(path: &Path) -> Result<arbor_graph::ArborGraph> {
    // Refuse to index an un-indexed project unless auto-indexing is enabled —
    // this is the choke point for read commands that don't call
    // ensure_arbor_initialized first, and prevents writing a cache into a
    // project the user never opted in to.
    if !project_is_indexed(path) && !auto_index_enabled() {
        return Err(not_indexed_error(path));
    }

    // When a bridge is running it keeps graph.bin fresh via its own persister —
    // skip the staleness check to avoid a redundant re-index that races with
    // the bridge's writes.
    let store_path = graph_store_path(path);
    let bridge_may_be_running = store_path.join("db").exists();

    let stale = !bridge_may_be_running && cache_is_stale(path);
    if !stale {
        if let Ok(graph) = load_graph_binary(path) {
            return Ok(graph);
        }

        if let Ok(graph) = load_graph_snapshot(path) {
            return Ok(graph);
        }
    }

    // Only try sled store if no snapshot files exist AND no bridge/server
    // could be holding a lock. Sled locks are exclusive — a running bridge
    // will cause CLI calls to block indefinitely.
    let has_snapshot_files = graph_binary_path(path).exists() || graph_snapshot_path(path).exists();

    if !has_snapshot_files && !bridge_may_be_running {
        if let Ok(graph) = load_graph_from_store(path) {
            let _ = save_graph_snapshot(path, &graph);
            let _ = save_graph_binary(path, &graph);
            return Ok(graph);
        }
    }

    let result = index_directory(path, IndexOptions::default())?;
    save_graph_snapshot(path, &result.graph)?;
    save_graph_binary(path, &result.graph)?;
    Ok(result.graph)
}

pub(crate) fn git_changed_files(path: &Path) -> Result<Vec<String>> {
    list_changed_files(path).map_err(|e| e.into())
}

pub(crate) fn resolve_node_or_file_target(
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

pub(crate) fn command_exists(cmd: &str) -> bool {
    // Input validation to prevent command injection (CWE-78)
    if cmd.is_empty() || cmd.len() > 255 {
        return false;
    }
    // Only allow alphanumeric, hyphen, underscore, dot, slash, and backslash
    if !cmd
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/' || c == '\\')
    {
        return false;
    }

    Command::new(cmd)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub(crate) fn open_in_editor(file: &str, line: u32) -> Result<()> {
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

pub(crate) fn export_graph(graph: &arbor_graph::ArborGraph, path: &Path) -> Result<()> {
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

/// Suggest similar symbols when exact match fails
pub(crate) fn suggest_similar_symbols(graph: &arbor_graph::ArborGraph, target: &str) -> Result<()> {
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

/// Periodically persists the live graph to `graph.bin` while a bridge runs.
///
/// Only one bridge per project wins the persist lock — additional bridges for
/// the same project skip disk writes (their in-memory graph + MCP are still
/// fully functional). This prevents multiple bridges from stomping each other.
pub(crate) async fn run_graph_persister(
    mut rx: tokio::sync::broadcast::Receiver<arbor_server::BroadcastMessage>,
    graph: std::sync::Arc<tokio::sync::RwLock<arbor_graph::ArborGraph>>,
    path: PathBuf,
) {
    use arbor_server::BroadcastMessage;
    use fs2::FileExt;
    use tokio::sync::broadcast::error::RecvError;
    use tokio::time::{interval, Duration};

    // Acquire an exclusive advisory lock — only one persister per project.
    let lock_path = path.join(".arbor").join("persist.lock");
    let lock_file = match fs::File::create(&lock_path) {
        Ok(f) => f,
        Err(_) => return,
    };
    if lock_file.try_lock_exclusive().is_err() {
        eprintln!(
            "{} Another bridge is persisting for this project — skipping disk writes",
            "ℹ".cyan()
        );
        return;
    }

    const FLUSH_SECS: u64 = 5;
    let mut dirty = false;
    let mut tick = interval(Duration::from_secs(FLUSH_SECS));

    loop {
        tokio::select! {
            msg = rx.recv() => match msg {
                Ok(BroadcastMessage::GraphUpdate(_)) => dirty = true,
                Ok(_) => {}
                Err(RecvError::Lagged(_)) => dirty = true,
                Err(RecvError::Closed) => break,
            },
            _ = tick.tick() => {
                if dirty {
                    let guard = graph.read().await;
                    if let Err(e) = save_graph_binary(&path, &guard) {
                        eprintln!("{} Failed to persist graph cache: {}", "⚠".yellow(), e);
                    }
                    dirty = false;
                }
            }
        }
    }

    // Lock released when lock_file drops (process exit or loop break).
    drop(lock_file);
}

/// Resolves a symbol name to the node a user most likely meant.
///
/// Names collide constantly — `getPath` is a utility function in `utils/url.ts`
/// *and* a method on four AWS Lambda event processors. This used to take
/// `find_by_name(..).first()`, i.e. whichever file happened to be parsed first,
/// and then answered as if that were the only candidate. On hono that meant
/// `arbor inspect getPath` reported "unreachable, 0 callers, may be dead code"
/// about a function 23 files depend on.
///
/// Candidates are ranked by graph degree, then centrality, then file path, so
/// the pick is both meaningful and deterministic. Alternatives are returned so
/// the caller can tell the user what else matched.
pub(crate) fn resolve_symbol_ranked(
    graph: &arbor_graph::ArborGraph,
    symbol: &str,
) -> Result<(arbor_graph::NodeId, Vec<arbor_graph::NodeId>)> {
    let mut candidates = graph.resolve_symbol_ranked(symbol);
    if candidates.is_empty() {
        return Err(format!("Symbol '{}' not found", symbol).into());
    }
    let best = candidates.remove(0);
    Ok((best, candidates))
}

pub(crate) fn resolve_symbol(
    graph: &arbor_graph::ArborGraph,
    symbol: &str,
) -> Result<arbor_graph::NodeId> {
    resolve_symbol_ranked(graph, symbol).map(|(best, _)| best)
}

/// Tells the user which definition was chosen when a name matched several.
///
/// Silence here is what turned an ambiguous lookup into a confidently wrong
/// answer, so the note is printed even though it adds noise.
pub(crate) fn report_symbol_ambiguity(
    graph: &arbor_graph::ArborGraph,
    symbol: &str,
    chosen: arbor_graph::NodeId,
    others: &[arbor_graph::NodeId],
) {
    if others.is_empty() {
        return;
    }
    let describe = |i: arbor_graph::NodeId| {
        graph
            .get(i)
            .map(|n| {
                format!(
                    "{} ({}) {}:{}",
                    n.qualified_name, n.kind, n.file, n.line_start
                )
            })
            .unwrap_or_default()
    };

    eprintln!(
        "note: '{}' matches {} definitions; showing the most connected one:",
        symbol,
        others.len() + 1
    );
    eprintln!("      → {}", describe(chosen));
    for &o in others.iter().take(4) {
        eprintln!("        {}", describe(o));
    }
    if others.len() > 4 {
        eprintln!("        … and {} more", others.len() - 4);
    }
    eprintln!("      Pass a qualified name (e.g. Class.method) to pick a specific one.");
}

#[cfg(test)]
mod tests {
    use super::command_exists;

    #[test]
    fn test_command_exists_validation() {
        assert!(!command_exists("nonexistent-editor-binary-name"));
        assert!(!command_exists("sub-dir/another-editor"));
        assert!(!command_exists("bin\\editor.exe"));
        assert!(!command_exists("code; rm -rf /"));
        assert!(!command_exists("cursor & echo pwned"));
        assert!(!command_exists("nvim | cat /etc/passwd"));
        assert!(!command_exists("vim && whoami"));
        assert!(!command_exists("code $(rm -rf)"));
        assert!(!command_exists("code `rm -rf`"));
        assert!(!command_exists("code > file.txt"));
        assert!(!command_exists("code < file.txt"));
        assert!(!command_exists("code 2>&1"));
        let long_input = "a".repeat(300);
        assert!(!command_exists(&long_input));
    }
}
