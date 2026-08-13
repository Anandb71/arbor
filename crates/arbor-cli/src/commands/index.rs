use super::*;
use arbor_core::parse_file;
use arbor_graph::node_matches_changed_file;
use arbor_watcher::{index_directory, IndexOptions};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;
use std::time::Duration;

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

    let _ = init_arbor_dir(&resolved_path)?;

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
    let was_initialized = init_arbor_dir(&resolved_path)?;
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
        eprintln!("  - File extensions match the languages Arbor supports in this project (see `arbor status` for a list)");
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
