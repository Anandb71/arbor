<p align="center">
  <img src="https://raw.githubusercontent.com/Anandb71/arbor/main/docs/assets/arbor-logo.svg" alt="Arbor" width="80" height="80" />
</p>

<h1 align="center">arbor-graph-cli</h1>

<p align="center">
  <strong>The command-line interface for Arbor</strong><br>
  <em>Index your code. Query the graph. Navigate with AI.</em>
</p>

<p align="center">
  <a href="https://crates.io/crates/arbor-graph-cli"><img src="https://img.shields.io/crates/v/arbor-graph-cli?style=flat-square&color=blue" alt="Crates.io" /></a>
  <a href="https://github.com/Anandb71/arbor"><img src="https://img.shields.io/badge/repo-arbor-green?style=flat-square" alt="Repo" /></a>
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License" />
</p>

---

## What is Arbor?

Arbor is the **graph-native intelligence layer for code**. It parses your codebase into an AST graph where every function, class, and variable is a node, and every call, import, and inheritance is an edge.

This CLI is the primary interface for indexing, querying, and connecting your code to AI via the Model Context Protocol (MCP).

## Installation

```bash
cargo install arbor-graph-cli
```

crates.io can lag the GitHub releases. Prebuilt binaries and a from-source install of the latest tag are in the [installation guide](https://github.com/Anandb71/arbor/blob/main/docs/INSTALL.md).

## Quick Start

```bash
# One-shot setup in your project
cd your-project
arbor setup

# Run health diagnostics
arbor doctor

# Start the AI bridge + visualizer
arbor bridge --viz
```

## Commands

| Command | Description |
|---------|-------------|
| `arbor setup` | One-shot setup (init + index) |
| `arbor init` | Creates `.arbor/` config directory |
| `arbor index` | Full index of the codebase |
| `arbor index --changed-only` | Incremental index of git-modified files |
| `arbor query <q>` | Search the graph |
| `arbor map` | Ranked, token-budgeted project skeleton |
| `arbor callers <symbol>` / `arbor callees <symbol>` | One-hop traversal; same-named symbols are listed per definition |
| `arbor inspect <symbol>` | Full detail for one symbol |
| `arbor path <a> <b>` | Shortest call-graph path |
| `arbor entry-points` | HTTP handlers, main, jobs, webhooks, CLI commands |
| `arbor file-graph <path>` | Symbols and call edges within one file |
| `arbor diff` | Blast radius of git changes, per symbol (`--base <ref>`, `--staged`) |
| `arbor check` | CI safety gate for risky change sets (`--max-blast-radius N`) |
| `arbor summary` | Pull request description from the change set |
| `arbor agent review` / `onboard` / `guard` | Built-in agent workflows |
| `arbor receipt list` / `show` / `undo` | Plain-English receipts of what each agent turn changed |
| `arbor hook claude` | Wire Arbor into Claude Code (directives and per-turn receipts) |
| `arbor open <symbol>` | Open symbol/file in your editor |
| `arbor refactor <symbol>` | Blast-radius preview before refactoring |
| `arbor explain <symbol>` | Graph-backed context for code explanation |
| `arbor audit <sink>` | Security path tracing to sensitive sinks |
| `arbor serve` | Start the WebSocket server |
| `arbor export` | Export graph to JSON |
| `arbor status` | Show index statistics |
| `arbor watch` | Continuous re-index on file changes |
| `arbor bridge` | Start MCP server for AI integration |
| `arbor bridge --http` | MCP over stateless HTTP (port 3333) |
| `arbor bridge --viz` | MCP + Visualizer together |
| `arbor viz` | Launch the Logic Forest visualizer |
| `arbor gui` | Launch native Arbor GUI |
| `arbor pr-summary` | Generate impact summary for pull requests |
| `arbor doctor` (`check-health`) | System diagnostics |

## CI and Team Use

```bash
# Incremental refresh
arbor index --changed-only

# Pull-request safety checks
arbor diff
arbor check --json --max-blast-radius 30
```

## Docs

- [Quickstart](https://github.com/Anandb71/arbor/blob/main/docs/QUICKSTART.md)
- [Installation](https://github.com/Anandb71/arbor/blob/main/docs/INSTALL.md)
- [MCP Integration](https://github.com/Anandb71/arbor/blob/main/docs/MCP_INTEGRATION.md)
- [Receipts](https://github.com/Anandb71/arbor/blob/main/docs/RECEIPTS.md)
- [Changelog](https://github.com/Anandb71/arbor/blob/main/CHANGELOG.md)

## Supported Languages

Rust, TypeScript, JavaScript, Python, Go, Java, C, C++, C#, Dart, Kotlin, Swift, Ruby, PHP, Shell

## Links

- **Main Repository**: [github.com/Anandb71/arbor](https://github.com/Anandb71/arbor)
- **Documentation**: [docs/](https://github.com/Anandb71/arbor/tree/main/docs)
- **Glama MCP Directory**: [glama.ai/mcp/servers/@Anandb71/arbor](https://glama.ai/mcp/servers/@Anandb71/arbor)
