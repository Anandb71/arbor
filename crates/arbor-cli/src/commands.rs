//! CLI command implementations.

use arbor_graph::compute_centrality;
use arbor_server::{ArborServer, ServerConfig};
use arbor_watcher::index_directory;
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
pub fn index(path: &Path, output: Option<&Path>) -> Result<()> {
    println!("{}", "Indexing codebase...".cyan());

    let spinner = ProgressBar::new_spinner();
    spinner.set_style(ProgressStyle::default_spinner().template("{spinner:.cyan} {msg}")?);
    spinner.enable_steady_tick(Duration::from_millis(80));
    spinner.set_message("Scanning files...");

    let result = index_directory(path)?;

    spinner.finish_and_clear();

    // Print results
    println!(
        "{} Indexed {} files ({} nodes) in {}ms",
        "✓".green(),
        result.files_indexed.to_string().cyan(),
        result.nodes_extracted.to_string().cyan(),
        result.duration_ms
    );

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
    let result = index_directory(&path)?;

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
pub async fn serve(port: u16, headless: bool, path: &Path) -> Result<()> {
    let bind_addr = if headless { "0.0.0.0" } else { "127.0.0.1" };

    if headless {
        println!("{}", "Starting Arbor server in headless mode...".cyan());
    } else {
        println!("{}", "Starting Arbor server...".cyan());
    }

    // Index the codebase first
    let result = index_directory(path)?;
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
pub async fn viz(path: &Path) -> Result<()> {
    println!("{}", "Starting Arbor Visualizer stack...".cyan());

    // 1. Index Codebase
    let result = index_directory(path)?;
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
    let rpc_port = 7432;
    let rpc_addr = format!("127.0.0.1:{}", rpc_port).parse()?;
    let rpc_config = ServerConfig { addr: rpc_addr };
    let arbor_server = ArborServer::new(graph, rpc_config);
    let shared_graph = arbor_server.graph();

    // 3. Start Sync Server (WebSocket Broadcast)
    let sync_port = 8080;
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
    let viz_dir = path.join("visualizer");
    if viz_dir.exists() {
        println!("{}", "Launching Flutter Visualizer...".cyan());

        #[cfg(target_os = "windows")]
        let cmd = "flutter.bat";
        #[cfg(not(target_os = "windows"))]
        let cmd = "flutter";

        let status = std::process::Command::new(cmd)
            .arg("run")
            .arg("-d")
            .arg("windows")
            .current_dir(&viz_dir)
            .status();

        match status {
            Ok(_) => println!("Visualizer closed."),
            Err(e) => println!("Failed to launch visualizer: {}", e),
        }
    } else {
        println!(
            "{}",
            "Visualizer source not found in target directory.".yellow()
        );
        println!("Please run 'arbor viz' from the root of the arbor repo, or start the visualizer manually.");
    }

    Ok(())
}

/// Export the graph to JSON.
pub fn export(path: &Path, output: &Path) -> Result<()> {
    let result = index_directory(path)?;
    export_graph(&result.graph, output)?;
    Ok(())
}

/// Show index status.
pub fn status(path: &Path) -> Result<()> {
    let arbor_dir = path.join(".arbor");

    if !arbor_dir.exists() {
        println!("{} Arbor not initialized in this directory", "✗".red());
        println!("  Run {} to initialize", "arbor init".cyan());
        return Ok(());
    }

    // Quick index to get stats
    let result = index_directory(path)?;

    println!("{}", "Arbor Status".cyan().bold());
    println!();
    println!("  {} {}", "Files:".dimmed(), result.files_indexed);
    println!("  {} {}", "Nodes:".dimmed(), result.nodes_extracted);
    println!("  {} {}", "Edges:".dimmed(), result.graph.edge_count());
    println!("  {} TypeScript, Rust, Python", "Languages:".dimmed());

    Ok(())
}

/// Start the Agentic Bridge (MCP + Viz).
pub async fn bridge(path: &Path, launch_viz: bool) -> Result<()> {
    use arbor_mcp::McpServer;

    eprintln!("{} Arbor Bridge (MCP Mode)", "🔗".bold().cyan());

    // 1. Create Shared Graph (Empty initially)
    let graph = arbor_graph::ArborGraph::new();
    let shared_graph = std::sync::Arc::new(tokio::sync::RwLock::new(graph));

    // 2. Spawn Background Indexer
    let index_path = path.to_path_buf();
    let index_graph = shared_graph.clone();

    eprintln!("{} Starting background indexer...", "⏳".yellow());
    tokio::spawn(async move {
        // Run blocking indexer in spawn_blocking
        let result = tokio::task::spawn_blocking(move || index_directory(&index_path)).await;

        match result {
            Ok(Ok(index_result)) => {
                let mut guard = index_graph.write().await;
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
            Ok(Err(e)) => eprintln!("{} Indexing failed: {}", "⚠".red(), e),
            Err(e) => eprintln!("{} Indexer panicked: {}", "⚠".red(), e),
        }
    });

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
            let cmd = "flutter.bat";
            #[cfg(not(target_os = "windows"))]
            let cmd = "flutter";

            // Spawn visualizer in background
            std::process::Command::new(cmd)
                .arg("run")
                .arg("-d")
                .arg("windows")
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
    let result = index_directory(&path)?;
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
            return Err(format!("Node '{}' not found in graph", target).into());
        }
    };

    // Run impact analysis
    let analysis = graph.analyze_impact(node_idx, max_depth);

    if json_output {
        // JSON output
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
    } else {
        // Human-readable output
        println!("{}", "⚠️  Blast Radius".yellow().bold());
        println!(
            "Target: {} ({})",
            analysis.target.name.cyan(),
            analysis.target.kind
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
        let distant: Vec<_> = analysis
            .all_affected()
            .into_iter()
            .filter(|n| n.severity == arbor_graph::ImpactSeverity::Distant)
            .collect();

        println!(
            "Total: {} nodes (direct: {}, transitive: {}, distant: {})",
            analysis.total_affected.to_string().bold(),
            direct.len().to_string().red(),
            transitive.len().to_string().yellow(),
            distant.len().to_string().dimmed()
        );
        println!();

        if !direct.is_empty() {
            println!("{}", "Direct (1 hop):".red());
            for node in direct.iter().take(10) {
                print!("  • {} ({})", node.node_info.name, node.node_info.kind);
                if show_why {
                    print!(
                        " — {} {}",
                        node.entry_edge.to_string().dimmed(),
                        analysis.target.name
                    );
                }
                println!();
            }
            if direct.len() > 10 {
                println!("  ... and {} more", direct.len() - 10);
            }
            println!();
        }

        if !transitive.is_empty() {
            println!("{}", "Transitive (2-3 hops):".yellow());
            for node in transitive.iter().take(5) {
                print!("  • {}", node.node_info.name);
                if show_why {
                    print!(
                        " — {} hops via {}",
                        node.hop_distance,
                        node.entry_edge.to_string().dimmed()
                    );
                }
                println!();
            }
            if transitive.len() > 5 {
                println!("  ... and {} more", transitive.len() - 5);
            }
            println!();
        }

        if !distant.is_empty() {
            println!("{}", "Distant (4+ hops):".dimmed());
            println!("  {} nodes at depth 4+", distant.len());
        }

        println!();
        println!("Query time: {}ms", analysis.query_time_ms);
    }

    Ok(())
}

/// Explain code using graph-backed context.
pub fn explain(question: &str, max_tokens: usize, show_why: bool, json_output: bool) -> Result<()> {
    // Load the graph by indexing current directory
    let path = std::env::current_dir()?;
    let result = index_directory(&path)?;
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
