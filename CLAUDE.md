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

**`arbor-mcp`** — MCP stdio bridge for AI agents. Eleven tools in three tiers:
- **Orientation**: `get_map` — ranked, token-budgeted skeleton of the codebase (recommended first call)
- **Surgical**: `list_entry_points`, `get_callers`, `get_callees`, `search_symbols`, `get_file_graph`, `get_node_detail`
- **Broad**: `get_logic_path`, `get_knowledge_path`, `find_path`, `analyze_impact`

All tools emit a standard JSON envelope: `{ok, tool, arbor_version, data, meta: {node_count, suggested_next_tool, suggested_next_args}}`. Error responses use `{ok: false, error}`. Run via `arbor bridge`.

**`arbor-cli`** — Clap CLI with ~25 subcommands. All command logic lives in `src/commands.rs`. Entry point: `src/main.rs`. Dispatches to the other crates. Binary name: `arbor` (crate name: `arbor-graph-cli`).
Key features:
- `map . --exclude-test`: ranked, token-budgeted project skeleton (PageRank + entry point detection). Supports `--tokens N`, `--focus "pattern"`, `--focus-changed`, `--json`, `--verbose`.
- `callers`/`callees`/`entry-points`/`file-graph`/`inspect`/`path`: graph query commands matching MCP tools.
- `query "term1|term2" . --exclude-test`: multi-term OR search with test file filtering.
- `diff . --markdown`: formats impact analysis report as color-coded Markdown.
- `check . --markdown`: executes safety threshold validation and prints Markdown PASS/FAIL status.
- `summary .`: auto-generates structured Pull Request descriptions based on graph diff analysis.

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
- **Centrality persistence**: `arbor map` computes PageRank on first call and saves it to the binary cache. Subsequent calls skip recomputation (~0.5s vs ~1.5s).
- **Atomic cache writes**: `save_graph_binary`/`save_graph_snapshot` write to `.tmp` then atomically rename, preventing concurrent processes from reading half-written caches.
- **Sled lock avoidance**: CLI commands skip the sled store path if `cache/db` exists (implies a bridge may hold the exclusive lock). Falls back to re-indexing from source.
- **Whitespace-only diff filtering**: `git_changed_files()` cross-references `--name-status` against `--numstat` to exclude files with only whitespace changes.

### `.arbor/` Directory

Local cache created by `arbor init`/`arbor setup`. Contains `config.json` with default settings, `graph.bin` (bincode-serialized graph with centrality scores), and `graph.json` (JSON snapshot). Treated as workspace root marker alongside `.git`, `Cargo.toml`, `package.json`, `go.mod`, `pyproject.toml`.

## CLI Command Reference

| Command | Purpose |
|---|---|
| `setup .` | One-shot init + index |
| `init .` | Initialize `.arbor/` config |
| `index .` | Parse and build graph |
| `map . --exclude-test` | Ranked project skeleton (token-budgeted) |
| `query "name" .` | Fuzzy symbol search (supports `\|` for OR) |
| `callers "sym" .` | Who calls this? |
| `callees "sym" .` | What does this call? |
| `entry-points .` | HTTP handlers, main, jobs, webhooks |
| `file-graph "path" .` | Symbols + edges in one file |
| `inspect "sym" .` | Full symbol detail |
| `path "a" "b" .` | Shortest call-graph path |
| `diff .` | Blast radius of git changes |
| `check .` | CI safety threshold check |
| `refactor "sym" .` | Blast radius of changing a symbol |
| `summary .` | Auto-generate PR description |
| `bridge .` | Start MCP stdio server |
| `serve .` | Start WebSocket server |
| `watch .` | File watcher + auto re-index |

All query commands support `--json`. `map` additionally supports `--tokens N`, `--focus "pattern"`, `--focus-changed`, `--verbose`.

## Agent Integration (Claude Code)

To integrate arbor into a target project for AI agent use:

### 1. MCP server (`.mcp.json` at project root)

```json
{
  "mcpServers": {
    "arbor": {
      "command": "/Users/<you>/.cargo/bin/arbor",
      "args": ["bridge", "/absolute/path/to/project"]
    }
  }
}
```

### 2. Hooks (`.claude/settings.json`)

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "echo \"$CLAUDE_TOOL_INPUT\" | grep -q '\\barbor\\b' && [ ! -d .arbor ] && arbor init . >/dev/null 2>&1; exit 0"
          },
          {
            "type": "command",
            "command": "echo \"$CLAUDE_TOOL_INPUT\" | grep -qE '(grep|rg)\\s+(-[a-zA-Z]*r|-[a-zA-Z]*R|--recursive)' && echo 'BLOCK: Use arbor instead of recursive grep.' && exit 1; exit 0"
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "FLAG=\".arbor/.map-injected-$(date +%Y%m%d)\"; [ -f \"$FLAG\" ] && exit 0; touch \"$FLAG\"; echo '--- arbor map (project skeleton) ---'; arbor map . --exclude-test 2>/dev/null; echo '--- end arbor map ---'; exit 0"
          }
        ]
      }
    ]
  }
}
```

**What these do:**
- **PreToolUse #1**: Auto-initializes `.arbor/` if an arbor command is called but the project isn't set up yet.
- **PreToolUse #2**: Blocks recursive grep/ripgrep and tells the agent to use arbor instead.
- **PostToolUse**: Injects `arbor map` output (project skeleton) on the first Bash call each day. Flag file is per-project (`.arbor/.map-injected-<date>`), so each project triggers independently.

### 3. Permissions (`.claude/settings.local.json`)

```json
{
  "permissions": {
    "allow": ["Bash(arbor *)"]
  }
}
```

### 4. Agent instructions (`CLAUDE.md` at project root)

Document the workflow: map is auto-injected, use `arbor query`/`callers`/`callees`/`file-graph` for navigation, only `Read` after arbor identifies the target file+line.

## Active Development Areas

- `cli-enhancements` branch: map command, CLI parity with MCP, concurrency fixes
- `arborv2/` — Tauri v2 desktop companion (in progress, excluded from workspace)
- `lattice/` — Lattice Personal OS integration (excluded from workspace)
- `extensions/arbor-vscode/` — VS Code extension (separate npm project)
- `visualizer/` — Flutter visualizer (separate Dart project)
