//! Arbor CLI - Command-line interface for Arbor
//!
//! This is the main entry point for users interacting with Arbor.
//! It provides commands for indexing, querying, and serving the code graph.

use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod audit;
mod commands;
mod hook;

#[derive(Parser)]
#[command(name = "arbor")]
#[command(author = "Arbor Contributors")]
#[command(version)]
#[command(about = "The Graph-Native Intelligence Layer for Code", long_about = None)]
struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// One-shot setup (init + index)
    Setup {
        /// Path to set up (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Follow symbolic links when walking directories
        #[arg(long)]
        follow_symlinks: bool,

        /// Disable caching (force full re-index)
        #[arg(long)]
        no_cache: bool,
    },

    /// Initialize Arbor in the current directory
    Init {
        /// Path to initialize (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Index the codebase and build the graph
    Index {
        /// Path to index (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Index only files changed in git (faster incremental refresh)
        #[arg(long)]
        changed_only: bool,

        /// Output file for the graph JSON
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Follow symbolic links when walking directories
        #[arg(long)]
        follow_symlinks: bool,

        /// Disable caching (force full re-index)
        #[arg(long)]
        no_cache: bool,
    },

    /// Search the code graph
    Query {
        /// Search query
        query: String,

        /// Path to index/search (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Maximum results to return
        #[arg(short, long, default_value = "10")]
        limit: usize,

        /// Exclude test/spec/fixture/mock files from results
        #[arg(long)]
        exclude_test: bool,
    },

    /// Analyze git changes and preview impact blast radius
    Diff {
        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Maximum impact traversal depth
        #[arg(short, long, default_value = "5")]
        depth: usize,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Output as Markdown (for PR comments)
        #[arg(long)]
        markdown: bool,
    },

    /// CI safety mode for changed code paths
    Check {
        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Maximum impact traversal depth
        #[arg(short, long, default_value = "5")]
        depth: usize,

        /// Blast radius threshold considered risky
        #[arg(long, default_value = "25")]
        max_blast_radius: usize,

        /// Do not fail with non-zero exit code on risky changes
        #[arg(long)]
        no_fail: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Output as Markdown (for PR comments)
        #[arg(long)]
        markdown: bool,
    },

    /// Start the Arbor server
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "7432")]
        port: u16,

        /// Headless mode: bind to 0.0.0.0 for remote access (WSL/Docker/Server)
        #[arg(long)]
        headless: bool,

        /// Path to index (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Follow symbolic links when walking directories
        #[arg(long)]
        follow_symlinks: bool,
    },

    /// Export the graph to JSON
    Export {
        /// Output file
        #[arg(short, long, default_value = "arbor-graph.json")]
        output: PathBuf,

        /// Path to index (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Show index status and statistics
    Status {
        /// Path to check (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// List all indexed files
        #[arg(long)]
        files: bool,
    },

    /// Start the Arbor Visualizer
    Viz {
        /// Path to visualize (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Follow symbolic links when walking directories
        #[arg(long)]
        follow_symlinks: bool,
    },

    /// Start the Agentic Bridge (MCP + Viz)
    Bridge {
        /// Path to index (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Also launch the Flutter visualizer
        #[arg(long)]
        viz: bool,

        /// Follow symbolic links when walking directories
        #[arg(long)]
        follow_symlinks: bool,
    },

    /// Check system health and environment
    #[command(visible_alias = "check-health")]
    Doctor {
        /// Path to diagnose (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Preview blast radius before refactoring a node
    Refactor {
        /// The node to analyze (function name, class name, or qualified path)
        target: String,

        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Maximum depth to search (default: 5)
        #[arg(short, long, default_value = "5")]
        depth: usize,

        /// Show detailed reasoning for each affected node
        #[arg(long)]
        why: bool,

        /// Output as JSON instead of formatted text
        #[arg(long)]
        json: bool,
    },

    /// Explain code using graph-backed context
    Explain {
        /// The question or code path to explain
        question: String,

        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Maximum tokens for context (default: 4000)
        #[arg(short, long, default_value = "4000")]
        tokens: usize,

        /// Show detailed reasoning for context selection
        #[arg(long)]
        why: bool,

        /// Output as JSON instead of formatted text
        #[arg(long)]
        json: bool,
    },

    /// Open a symbol location in your editor
    Open {
        /// Symbol name, qualified id, or file path
        symbol: String,

        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Launch the graphical interface
    Gui {
        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Generate a PR summary for refactored symbols
    PrSummary {
        /// Symbols that were changed (comma-separated)
        symbols: String,

        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Generate an auto-description for a PR based on graph changes
    Summary {
        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Watch for file changes and re-index automatically
    Watch {
        /// Path to watch (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Security audit: Trace paths to sensitive sinks
    Audit {
        /// The sensitive sink to analyze (e.g., "db_query", "exec")
        sink: String,

        /// Maximum depth to search (default: 8)
        #[arg(short, long, default_value = "8")]
        depth: usize,

        /// Output format (default: text, options: json, csv)
        #[arg(long, default_value = "text")]
        format: String,

        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Show direct callers of a symbol (who calls this?)
    Callers {
        /// The symbol to look up (function name, class name, or qualified path)
        symbol: String,

        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show direct callees of a symbol (what does this call?)
    Callees {
        /// The symbol to look up
        symbol: String,

        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List all detected entry points (HTTP handlers, main, webhooks, jobs, CLI commands)
    EntryPoints {
        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show symbols and call edges within a single file
    FileGraph {
        /// Relative path to the file (e.g., 'src/auth.rs')
        file: String,

        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show full detail for a single symbol
    Inspect {
        /// Name or ID of the symbol
        symbol: String,

        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Find the shortest path between two symbols in the call graph
    #[command(name = "path")]
    FindPath {
        /// Start symbol (name or ID)
        start: String,

        /// End symbol (name or ID)
        end: String,

        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Install Arbor directives + hooks into a coding-agent harness
    Hook {
        /// Harness to wire up (currently: claude)
        #[arg(default_value = "claude")]
        harness: String,

        /// Project path to install into (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Install into the user's global config instead of the project
        #[arg(long)]
        global: bool,
    },

    /// Output a ranked, token-budgeted skeleton of the codebase
    Map {
        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Token budget (output will not exceed this estimate)
        #[arg(long, default_value = "1024")]
        tokens: usize,

        /// Exclude test/spec/fixture/mock files
        #[arg(long)]
        exclude_test: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Show full relative paths (disable path compression)
        #[arg(long)]
        verbose: bool,

        /// Boost symbols in files changed vs HEAD
        #[arg(long)]
        focus_changed: bool,

        /// Boost symbols in files matching this glob pattern (e.g. "*/service/*")
        #[arg(long)]
        focus: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Set up logging
    let filter = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_target(false),
        )
        .with(tracing_subscriber::EnvFilter::new(filter))
        .init();

    let result = match cli.command {
        Commands::Setup {
            path,
            follow_symlinks,
            no_cache,
        } => commands::init(&path)
            .and_then(|_| commands::index(&path, None, follow_symlinks, no_cache, false)),
        Commands::Init { path } => commands::init(&path),
        Commands::Index {
            path,
            changed_only,
            output,
            follow_symlinks,
            no_cache,
        } => commands::index(
            &path,
            output.as_deref(),
            follow_symlinks,
            no_cache,
            changed_only,
        ),
        Commands::Query {
            query,
            path,
            limit,
            exclude_test,
        } => commands::query(&query, limit, &path, exclude_test),
        Commands::Diff {
            path,
            depth,
            json,
            markdown,
        } => commands::diff(&path, depth, json, markdown),
        Commands::Check {
            path,
            depth,
            max_blast_radius,
            no_fail,
            json,
            markdown,
        } => commands::check(&path, depth, max_blast_radius, no_fail, json, markdown),
        Commands::Serve {
            port,
            headless,
            path,
            follow_symlinks,
        } => commands::serve(port, headless, &path, follow_symlinks).await,
        Commands::Export { output, path } => commands::export(&path, &output),
        Commands::Status { path, files } => commands::status(&path, files),
        Commands::Viz {
            path,
            follow_symlinks,
        } => commands::viz(&path, follow_symlinks).await,
        Commands::Bridge {
            path,
            viz,
            follow_symlinks,
        } => commands::bridge(&path, viz, follow_symlinks).await,
        Commands::Doctor { path } => commands::check_health(Some(&path)).await,
        Commands::Refactor {
            target,
            path,
            depth,
            why,
            json,
        } => commands::refactor(&target, depth, why, json, &path),
        Commands::Explain {
            question,
            path,
            tokens,
            why,
            json,
        } => commands::explain(&question, tokens, why, json, &path),
        Commands::Open { symbol, path } => commands::open(&symbol, &path),
        Commands::Gui { path } => commands::gui(&path),
        Commands::PrSummary { symbols, path } => commands::pr_summary(&symbols, &path),
        Commands::Summary { path } => commands::summary(&path),
        Commands::Watch { path } => commands::watch(&path).await,
        Commands::Audit {
            sink,
            depth,
            format,
            path,
        } => commands::audit(&sink, depth, &format, &path),
        Commands::Callers { symbol, path, json } => commands::callers(&symbol, &path, json),
        Commands::Callees { symbol, path, json } => commands::callees(&symbol, &path, json),
        Commands::EntryPoints { path, json } => commands::entry_points(&path, json),
        Commands::FileGraph { file, path, json } => commands::file_graph(&file, &path, json),
        Commands::Inspect { symbol, path, json } => commands::inspect(&symbol, &path, json),
        Commands::FindPath {
            start,
            end,
            path,
            json,
        } => commands::find_path_cmd(&start, &end, &path, json),
        Commands::Hook {
            harness,
            path,
            global,
        } => hook::run(&harness, &path, global),
        Commands::Map {
            path,
            tokens,
            exclude_test,
            json,
            verbose,
            focus_changed,
            focus,
        } => commands::map(
            &path,
            tokens,
            exclude_test,
            json,
            verbose,
            focus_changed,
            focus.as_deref(),
        ),
    };

    if let Err(e) = result {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}
