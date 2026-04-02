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
  <a href="https://glama.ai/mcp/servers/Anandb71/arbor"><img src="https://glama.ai/mcp/servers/Anandb71/arbor/badges/score.svg" alt="Glama Score" /></a>
  <a href="https://skillsplayground.com/mcps/nandb71-arbor/"><img src="https://skillsplayground.com/badges/mcp/nandb71-arbor.svg" alt="Skills Playground" /></a>
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License" />
</p>

---

## Overview

`arbor-mcp` is the **AI Bridge** for [Arbor](https://github.com/Anandb71/arbor). It implements the [Model Context Protocol](https://modelcontextprotocol.io/) to let LLMs like Claude Desktop navigate your codebase as a graph.

Compared with typical code-intel MCP servers, Arbor focuses on:

- **Graph-backed impact analysis** (not keyword-only retrieval)
- **Explainable confidence and architectural role classification**
- **Git-aware CI workflows** (`arbor diff`, `arbor check`) for change risk gating

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

### Claude Code (recommended)

```bash
claude mcp add --transport stdio --scope project arbor -- arbor bridge
claude mcp list
```

Inside Claude Code, run:

```text
/mcp
```

### Cursor / VS Code / Claude Desktop

See full multi-client setup in [`docs/MCP_INTEGRATION.md`](../../docs/MCP_INTEGRATION.md).

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
- **Official Registry API (exact lookup)**: https://registry.modelcontextprotocol.io/v0.1/servers?search=io.github.Anandb71/arbor

> If `github.com/mcp` search does not show Arbor yet, rely on the official registry API lookup above (authoritative source).
