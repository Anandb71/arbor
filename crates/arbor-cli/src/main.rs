//! Arbor CLI - Command-line interface for Arbor
//!
//! This is the main entry point for users interacting with Arbor.
//! It provides commands for indexing, querying, and serving the code graph.

use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod commands;

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

        /// Output file for the graph JSON
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Search the code graph
    Query {
        /// Search query
        query: String,

        /// Maximum results to return
        #[arg(short, long, default_value = "10")]
        limit: usize,
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
    },

    /// Start the Arbor Visualizer
    Viz {
        /// Path to visualize (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Start the Agentic Bridge (MCP + Viz)
    Bridge {
        /// Path to index (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Also launch the Flutter visualizer
        #[arg(long)]
        viz: bool,
    },

    /// Check system health and environment
    #[command(hide = true)]
    CheckHealth,

    /// Preview blast radius before refactoring a node
    Refactor {
        /// The node to analyze (function name, class name, or qualified path)
        target: String,

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
        Commands::Init { path } => commands::init(&path),
        Commands::Index { path, output } => commands::index(&path, output.as_deref()),
        Commands::Query { query, limit } => commands::query(&query, limit),
        Commands::Serve {
            port,
            headless,
            path,
        } => commands::serve(port, headless, &path).await,
        Commands::Export { output, path } => commands::export(&path, &output),
        Commands::Status { path } => commands::status(&path),
        Commands::Viz { path } => commands::viz(&path).await,
        Commands::Bridge { path, viz } => commands::bridge(&path, viz).await,
        Commands::CheckHealth => commands::check_health().await,
        Commands::Refactor {
            target,
            depth,
            why,
            json,
        } => commands::refactor(&target, depth, why, json),
        Commands::Explain {
            question,
            tokens,
            why,
            json,
        } => commands::explain(&question, tokens, why, json),
    };

    if let Err(e) = result {
        eprintln!("{} {}", "error:".red().bold(), e);
        std::process::exit(1);
    }
}
