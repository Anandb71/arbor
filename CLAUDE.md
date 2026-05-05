# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Build
cargo build --workspace

# Test all crates
cargo test --workspace

# Test single crate
cargo test -p arbor-graph
cargo test -p arbor-core

# Test single test by name
cargo test -p arbor-graph -- ranking::tests::test_pagerank_basic

# Lint
cargo clippy --workspace --all-targets --all-features

# Format check
cargo fmt --all -- --check

# Format fix
cargo fmt --all

# Release build (CLI binary)
cargo build --locked --release -p arbor-graph-cli

# Run CLI locally
cargo run -p arbor-graph-cli -- <command>
```

## Architecture

Arbor is a **semantic code graph engine** — it parses codebases into a dependency graph and exposes that graph to CLIs, GUIs, WebSocket clients, and AI agents (via MCP).

### Crate Dependency Order

```
arbor-core  →  arbor-graph  →  arbor-watcher
                     │                │
                     └───── arbor-server ─────┐
                                              │
                     arbor-mcp ───────────────┤
                     arbor-cli ───────────────┘
                     arbor-gui ──────────────────→ arbor-{core,graph,watcher}
```

### Crate Roles

**`arbor-core`** — Tree-sitter AST parsing. Extracts functions, classes, structs, imports, and call edges for 9 production languages (Rust, TS/JS, Python, Go, Java, C/C++, C#, Dart) plus 5 fallback parsers. Each language lives in `crates/arbor-core/src/languages/`. `parser_v2.rs` is the active parser; `parser.rs` is legacy.

**`arbor-graph`** — In-memory petgraph + sled persistence. Key modules:
- `builder.rs` — converts parsed nodes/edges into the graph, builds per-file import maps for cross-module edge filtering
- `ranking.rs` — PageRank with 10% weight for test-file callers
- `heuristics.rs` — entry point detection (main, HTTP routes, webhooks, jobs, CLI commands)
- `impact.rs` — blast radius, shortest path (A*)
- `slice.rs` — context trimming (token-aware, tiktoken)
- `symbol_table.rs` — cross-file FQN resolution
- `confidence.rs` — edge confidence scoring
- `store.rs` — sled-backed persistence

**`arbor-watcher`** — `notify`-based file watcher. Debounces at 100ms, respects `.gitignore`, triggers incremental re-parse of changed files only. Two-tier cache: file-level AST + node-level byte ranges.

**`arbor-server`** — Tokio WebSocket server on `ws://localhost:7432`. JSON-RPC methods: `discover`, `impact`, `context`, `graph.subscribe`, `spotlight`. RwLock-protected shared graph state.

**`arbor-mcp`** — MCP stdio bridge for AI agents. Ten tools in two tiers:
- **Surgical** (v2.1.0): `list_entry_points`, `get_callers`, `get_callees`, `search_symbols`, `get_file_graph`, `get_node_detail`
- **Broad** (existing): `get_logic_path`, `get_knowledge_path`, `find_path`, `analyze_impact`

All new tools emit a standard JSON envelope: `{ok, tool, arbor_version, data, meta: {node_count, suggested_next_tool, suggested_next_args}}`. Error responses use `{ok: false, error}`. Run via `arbor bridge`.

**`arbor-cli`** — Clap CLI with ~20 subcommands. All command logic lives in `src/commands.rs` (~83KB). Entry point: `src/main.rs`. Dispatches to the other crates. Binary name: `arbor` (crate name: `arbor-graph-cli`).

**`arbor-gui`** — egui immediate-mode desktop UI. Standalone binary.

### Data Flow

1. `arbor-core` parses files with Tree-sitter → `Node` + call edge structs
2. `arbor-graph`'s `builder.rs` assembles petgraph, builds import maps, filters false cross-module edges
3. `arbor-watcher` detects changes → partial re-parse → graph patch
4. `arbor-server` exposes live graph over WebSocket
5. `arbor-mcp` wraps graph queries as MCP tools for AI clients
6. `arbor-cli` orchestrates all of the above

### Key Design Decisions

- **Import-aware edge filtering**: Cross-file edges are dropped if the caller's file has explicit imports but does not import the callee's name. Prevents cross-module false positives.
- **Test-file-aware PageRank**: Callers from `test/spec/fixture/mock` files contribute 10% weight, not 100%, to avoid test-inflated centrality scores.
- **No dotted method calls in JS/TS**: `obj.method()` calls are excluded from edges (requires type inference we don't have); only bare `foo()` and `this./super.` calls are recorded.
- **Stack overflow prevention**: `stacker::maybe_grow` wraps recursive AST traversal in all parsers; `collect_calls` uses iterative `TreeCursor` to avoid deep call stacks.
- **Import nodes excluded from graph**: Import/import-from AST nodes are processed for import map data but never added as vertices (prevents false centrality).

### `.arbor/` Directory

Local cache created by `arbor init`/`arbor setup`. Contains `config.json` with default settings. Treated as workspace root marker alongside `.git`, `Cargo.toml`, `package.json`, `go.mod`, `pyproject.toml`.

## Active Development Areas

- `v2.0` branch: current development branch
- `arborv2/` — Tauri v2 desktop companion (in progress, excluded from workspace)
- `lattice/` — Lattice Personal OS integration (excluded from workspace)
- `extensions/arbor-vscode/` — VS Code extension (separate npm project)
- `visualizer/` — Flutter visualizer (separate Dart project)
