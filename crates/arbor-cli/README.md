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
| `arbor query <q>` | Search the graph |
| `arbor serve` | Start the WebSocket server |
| `arbor bridge` | Start MCP server for AI integration |
| `arbor bridge --viz` | MCP + Visualizer together |
| `arbor viz` | Launch the Logic Forest visualizer |
| `arbor doctor` (`check-health`) | System diagnostics |

## Supported Languages

Rust, TypeScript, JavaScript, Python, Go, Java, C, C++, C#, Dart

## Links

- **Main Repository**: [github.com/Anandb71/arbor](https://github.com/Anandb71/arbor)
- **Documentation**: [docs/](https://github.com/Anandb71/arbor/tree/main/docs)
- **MCP Registry**: `io.github.Anandb71/arbor`
