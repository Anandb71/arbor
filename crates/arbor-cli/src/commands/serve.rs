use super::*;
use arbor_graph::compute_centrality;
use arbor_mcp::is_git_repo;
use arbor_server::{ArborServer, ServerConfig};
use arbor_watcher::{index_directory, IndexOptions};
use colored::Colorize;
use std::fs;
use std::path::Path;

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
    graph.set_centrality_scores(scores);

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
    graph.set_centrality_scores(scores);

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

/// Start the Agentic Bridge (MCP + Viz).
pub async fn bridge(
    path: &Path,
    launch_viz: bool,
    follow_symlinks: bool,
    http: bool,
    http_port: u16,
) -> Result<()> {
    use arbor_mcp::{run_http_server, McpServer};
    use std::sync::Arc;

    let resolved_path = resolve_project_path(path)?;
    let _ = ensure_arbor_initialized(&resolved_path)?;

    eprintln!("{} Arbor Bridge (MCP Mode)", "🔗".bold().cyan());

    // 1. Create Shared Graph (Empty initially)
    let graph = arbor_graph::ArborGraph::new();
    let shared_graph = std::sync::Arc::new(tokio::sync::RwLock::new(graph));

    // 2. Index in background so MCP stdio starts immediately (prevents client timeout)
    let index_path = resolved_path.to_path_buf();
    let options = IndexOptions {
        follow_symlinks,
        cache_path: Some(resolved_path.join(".arbor").join("cache")),
    };
    eprintln!("{} Starting initial index (background)...", "⏳".yellow());

    let index_graph = shared_graph.clone();
    tokio::spawn(async move {
        let result =
            tokio::task::spawn_blocking(move || index_directory(&index_path, options)).await;
        match result {
            Ok(Ok(index_result)) => {
                let mut guard = index_graph.write().await;
                *guard = index_result.graph;

                let scores = compute_centrality(&guard, 20, 0.85);
                guard.set_centrality_scores(scores);

                eprintln!(
                    "{} Index Ready: {} files, {} nodes",
                    "✓".green(),
                    index_result.files_indexed,
                    index_result.nodes_extracted
                );
            }
            Ok(Err(e)) => eprintln!("{} Indexing failed: {}", "⚠".red(), e),
            Err(e) => eprintln!("{} Index task panicked: {}", "⚠".red(), e),
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

    // Persist the live graph to disk so cold `arbor map`/`query` reads stay fast
    // and fresh. The background indexer broadcasts on every patch; we debounce
    // those into at most one graph.bin write every few seconds.
    let persist_rx = sync_server.subscribe();
    let persist_graph = shared_graph.clone();
    let persist_path = resolved_path.to_path_buf();
    tokio::spawn(async move {
        run_graph_persister(persist_rx, persist_graph, persist_path).await;
    });

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

    // 4. Start MCP Server (Main Thread) WITH Spotlight capability
    // IMPORTANT: All logging MUST be to stderr from here on.
    let mcp = McpServer::with_spotlight_and_project(
        shared_graph,
        spotlight_handle,
        resolved_path.clone(),
    );

    if http {
        let mcp_http = Arc::new(mcp);
        let port = http_port;
        let http_mcp = mcp_http.clone();
        tokio::spawn(async move {
            if let Err(e) = run_http_server(http_mcp, port).await {
                eprintln!("MCP HTTP server error: {}", e);
            }
        });
        eprintln!(
            "{} MCP HTTP transport enabled on port {} (2026-07-28)",
            "✓".green(),
            http_port
        );
        mcp_http.run_stdio().await?;
    } else {
        mcp.run_stdio().await?;
    }

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
