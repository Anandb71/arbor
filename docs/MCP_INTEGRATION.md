# Arbor MCP Integration Guide

> Connect Arbor's code graph intelligence to AI agents via Model Context Protocol.

---

## What is the MCP Bridge?

Arbor's MCP (Model Context Protocol) bridge allows AI agents like Claude and Cursor to:

- **Query the code graph** — understand dependencies and relationships
- **Analyze impact** — see blast radius before refactoring
- **Find paths** — trace connections between any two symbols

The bridge communicates over **stdio** using JSON-RPC, following the [MCP specification](https://modelcontextprotocol.io/).

**Directory listing:** [Glama MCP Directory — Arbor](https://glama.ai/mcp/servers/@Anandb71/arbor)

---

## Setup for Cursor

Create or edit `.cursor/mcp.json` in your project root:

```json
{
  "mcpServers": {
    "arbor": {
      "command": "arbor",
      "args": ["bridge"],
      "cwd": "."
    }
  }
}
```

Then in Cursor:
1. Open Command Palette (Cmd+Shift+P)
2. Search "MCP: Reload Servers"
3. Arbor tools will appear in the AI assistant

---

## Setup for VS Code

VS Code now supports MCP server definitions via workspace config.

Create `.vscode/mcp.json`:

```json
{
  "servers": {
    "arbor": {
      "command": "arbor",
      "args": ["bridge"],
      "cwd": "${workspaceFolder}"
    }
  }
}
```

Then:
1. Open Command Palette
2. Run **MCP: List Servers**
3. Trust/approve the workspace prompt if shown
4. Verify Arbor tools are available in your MCP-enabled extension/chat workflow

> Tip: use workspace-scoped MCP config for repos and user-scoped config only for globally trusted tooling.

---

## Setup for Claude Desktop

Edit your Claude Desktop config file:

**macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`  
**Windows**: `%APPDATA%\Claude\claude_desktop_config.json`

```json
{
  "mcpServers": {
    "arbor": {
      "command": "arbor",
      "args": ["bridge"],
      "cwd": "/path/to/your/project"
    }
  }
}
```

Restart Claude Desktop to load the integration.

---

## Available Tools

| Tool | Description |
|------|-------------|
| `get_logic_path` | Traces call graph from a symbol |
| `analyze_impact` | Returns blast radius with confidence/roles |
| `find_path` | Finds shortest path between two symbols (when exposed by current server build) |

### Example: analyze_impact

**Input:**
```json
{
  "name": "analyze_impact",
  "arguments": {
    "node_id": "detect_language",
    "max_depth": 5
  }
}
```

**Output includes:**
- `confidence.level` — High/Medium/Low
- `confidence.reasons` — Why this confidence
- `role` — Entry Point, Core Logic, Utility, etc.
- `upstream` — Callers that would break
- `downstream` — Dependencies called
- `edges_explained` — Summary of connections

---

## Capabilities

The bridge advertises these capabilities to clients:

```json
{
  "streaming": false,
  "pagination": false,
  "json": true
}
```

---

## Known Limitations

1. **stdio only** — No WebSocket transport currently
2. **Single project** — Point `cwd` to your target project
3. **No hot reload** — Re-index after major changes (`arbor index`)
4. **Static analysis** — Dynamic dispatch marked as uncertain

---

## Troubleshooting

### "arbor: command not found"
Ensure Arbor is installed and in your PATH:
```bash
cargo install arbor-graph-cli
```

### MCP server not responding
Check that your project has been indexed:
```bash
cd /path/to/project
arbor setup
```

> Arbor auto-creates `.arbor/` for most commands, but `arbor setup` is the fastest reliable first-run path.

After significant branch updates, refresh incrementally:

```bash
arbor index --changed-only
```

### Tools not appearing in Cursor
1. Check `.cursor/mcp.json` syntax
2. Reload MCP servers from Command Palette
3. Run `arbor doctor` to verify local environment and ports
4. Check Cursor's MCP logs for errors

### "Node not found" errors
Use `arbor query <name>` to verify the symbol is indexed.

---

## Version

This guide is for Arbor releases with MCP capabilities (v1.6+). For branch/release channel policy, see [`CONTRIBUTING.md`](../CONTRIBUTING.md).
