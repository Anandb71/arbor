//! CLI command implementations.

use arbor_graph::compute_centrality;
use arbor_server::{ArborServer, ServerConfig};
use arbor_watcher::{index_directory, IndexOptions};
use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::fs;
use std::path::Path;
use std::time::Duration;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Initialize Arbor in a directory.
pub fn init(path: &Path) -> Result<()> {
    let arbor_dir = path.join(".arbor");

    if arbor_dir.exists() {
        println!("{} Already initialized", "✓".green());
        return Ok(());
    }

    fs::create_dir_all(&arbor_dir)?;

    // Create a default config file
    let config_path = arbor_dir.join("config.json");
    let default_config = serde_json::json!({
        "version": "1.0",
        "languages": ["typescript", "rust", "python"],
        "ignore": ["node_modules", "target", "dist", "__pycache__"]
    });

    fs::write(&config_path, serde_json::to_string_pretty(&default_config)?)?;

    println!("{} Initialized Arbor in {}", "✓".green(), path.display());
    println!("  Run {} to index your codebase", "arbor index".cyan());

    Ok(())
}

/// Index a directory and build the code graph.
pub fn index(
    path: &Path,
    output: Option<&Path>,
    follow_symlinks: bool,
    no_cache: bool,
) -> Result<()> {
    println!("{}", "Indexing codebase...".cyan());

    let spinner = ProgressBar::new_spinner();
    spinner.set_style(ProgressStyle::default_spinner().template("{spinner:.cyan} {msg}")?);
    spinner.enable_steady_tick(Duration::from_millis(80));
    spinner.set_message("Scanning files...");

    // Determine cache path
    let cache_path = if no_cache {
        None
    } else {
        Some(path.join(".arbor").join("cache"))
    };

    let options = IndexOptions {
        follow_symlinks,
        cache_path,
    };
    let result = index_directory(path, options)?;

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
        eprintln!("  - File extensions match supported languages (.rs, .ts, .py, .dart, .go)");
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

/// Query the code graph.
pub fn query(query: &str, limit: usize) -> Result<()> {
    // For now, we need to re-index. In a real implementation,
    // we'd load from a persisted graph or connect to the server.
    let path = std::env::current_dir()?;
    let result = index_directory(&path, IndexOptions::default())?;

    let matches: Vec<_> = result.graph.search(query).into_iter().take(limit).collect();

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

/// Start the Arbor server.
pub async fn serve(port: u16, headless: bool, path: &Path, follow_symlinks: bool) -> Result<()> {
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
    let result = index_directory(path, options)?;
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
    println!("{}", "Starting Arbor Visualizer stack...".cyan());

    // 1. Index Codebase
    let options = IndexOptions {
        follow_symlinks,
        cache_path: None,
    };
    let result = index_directory(path, options)?;
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
        watch_path: path.to_path_buf(),
        debounce_ms: 1000,
        extensions: vec![
            "ts".to_string(),
            "tsx".to_string(),
            "rs".to_string(),
            "py".to_string(),
            "dart".to_string(),
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
    let exe_dir = current_exe.parent().unwrap_or(path);

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
        let viz_dir = path.join("visualizer");
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
    let result = index_directory(path, IndexOptions::default())?;
    export_graph(&result.graph, output)?;
    Ok(())
}

/// Show index status.
pub fn status(path: &Path, show_files: bool) -> Result<()> {
    let arbor_dir = path.join(".arbor");

    if !arbor_dir.exists() {
        println!("{} Arbor not initialized in this directory", "✗".red());
        println!("  Run {} to initialize", "arbor init".cyan());
        return Ok(());
    }

    // Quick index to get stats
    let result = index_directory(path, IndexOptions::default())?;

    // Collect unique files from indexed nodes
    let files: std::collections::HashSet<_> = result
        .graph
        .nodes()
        .map(|n| n.file.clone())
        .collect();

    // Collect unique extensions from indexed files
    let extensions: std::collections::HashSet<_> = result
        .graph
        .nodes()
        .filter_map(|n| {
            std::path::Path::new(&n.file)
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_string())
        })
        .collect();
    let ext_list: Vec<_> = extensions.iter().take(10).collect();

    println!("{}", "📊 Arbor Status".cyan().bold());
    println!();
    println!("  {} {}", "Files indexed:".dimmed(), result.files_indexed);
    println!("  {} {}", "Nodes:".dimmed(), result.nodes_extracted);
    println!("  {} {}", "Edges:".dimmed(), result.graph.edge_count());
    println!(
        "  {} {}",
        "Extensions:".dimmed(),
        if ext_list.is_empty() {
            "(none)".to_string()
        } else {
            ext_list
                .iter()
                .map(|s| format!(".{}", s))
                .collect::<Vec<_>>()
                .join(", ")
        }
    );

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

    eprintln!("{} Arbor Bridge (MCP Mode)", "🔗".bold().cyan());

    // 1. Create Shared Graph (Empty initially)
    let graph = arbor_graph::ArborGraph::new();
    let shared_graph = std::sync::Arc::new(tokio::sync::RwLock::new(graph));

    // 2. Run Initial Index (Blocking)
    let index_path = path.to_path_buf();
    let options = IndexOptions {
        follow_symlinks,
        cache_path: Some(path.join(".arbor").join("cache")),
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
        watch_path: path.to_path_buf(),
        debounce_ms: 1000,
        extensions: vec![
            "rs".to_string(),
            "ts".to_string(),
            "py".to_string(),
            "dart".to_string(),
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
        let viz_dir = if path.join("visualizer").exists() {
            Some(path.join("visualizer"))
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
pub async fn check_health() -> Result<()> {
    use std::net::TcpListener;

    println!("{}", "🔍 Arbor Health Check".cyan().bold());
    println!("{}", "═".repeat(50));

    let mut all_ok = true;

    // Detect workspace root (if we're in crates/, go up one level)
    let workspace_root = if Path::new("Cargo.toml").exists() && Path::new("../visualizer").exists()
    {
        Path::new("..").to_path_buf()
    } else if Path::new("crates").exists() {
        Path::new(".").to_path_buf()
    } else {
        Path::new(".").to_path_buf()
    };

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
    } else {
        println!(
            "{} Arbor not initialized (run 'cargo run -- init' in workspace root)",
            "⚠".yellow()
        );
        all_ok = false;
    }

    println!("{}", "═".repeat(50));

    if all_ok {
        println!("{} All systems operational", "🚀".green().bold());
    } else {
        println!("{}", "⚠  Some checks require attention".yellow());
    }

    Ok(())
}

/// Preview blast radius before refactoring a node.
pub fn refactor(target: &str, max_depth: usize, show_why: bool, json_output: bool) -> Result<()> {
    // Load the graph by indexing current directory
    let path = std::env::current_dir()?;
    let result = index_directory(&path, IndexOptions::default())?;
    let graph = result.graph;

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
    // Relevance: 100 = exact name, 80 = exact suffix, 60 = starts with, 40 = contains
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
            continue; // No match
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
    suggestions.sort_by(|a, b| {
        b.1.cmp(&a.1).then_with(|| b.2.cmp(&a.2))
    });

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

/// Explain code using graph-backed context.
pub fn explain(question: &str, max_tokens: usize, show_why: bool, json_output: bool) -> Result<()> {
    // Load the graph by indexing current directory
    let path = std::env::current_dir()?;
    let result = index_directory(&path, IndexOptions::default())?;
    let graph = result.graph;

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

#[cfg(test)]
mod tests {
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
}
