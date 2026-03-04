<p align="center">
  <img src="https://raw.githubusercontent.com/Anandb71/arbor/main/docs/assets/arbor-logo.svg" alt="Arbor" width="60" height="60" />
</p>

<h1 align="center">arbor-mcp</h1>

<p align="center">
  <strong>Model Context Protocol server for Arbor</strong><br>
  <em>Let Claude walk your code graph</em>
</p>

<p align="center">
  <a href="https://crates.io/crates/arbor-mcp"><img src="https://img.shields.io/crates/v/arbor-mcp?style=flat-square&color=blue" alt="Crates.io" /></a>
  <a href="https://registry.modelcontextprotocol.io"><img src="https://img.shields.io/badge/MCP-registered-purple?style=flat-square" alt="MCP" /></a>
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License" />
</p>

---

## Overview

`arbor-mcp` is the **AI Bridge** for [Arbor](https://github.com/Anandb71/arbor). It implements the [Model Context Protocol](https://modelcontextprotocol.io/) to let LLMs like Claude Desktop navigate your codebase as a graph.

## MCP Tools

| Tool | Description |
|------|-------------|
| `get_logic_path` | Generate an architectural brief for a symbol |
| `find_path` | Shortest path between two nodes |
| `analyze_impact` | Predict blast radius of changes |

## Why MCP?

Instead of RAG-style "find similar text," Arbor lets the AI:

- **Walk the call graph** to understand control flow
- **Trace imports** to find the real source of a symbol
- **Predict impact** before making changes

## Usage

```bash
cargo install arbor-graph-cli
arbor setup
arbor bridge  # Starts MCP server over stdio
```

### Claude Desktop Config

```json
{
  "mcpServers": {
    "arbor": {
      "command": "arbor",
      "args": ["bridge"]
    }
  }
}
```

## Links

- **Main Repository**: [github.com/Anandb71/arbor](https://github.com/Anandb71/arbor)
- **MCP Registry**: `io.github.Anandb71/arbor`
