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
      "type": "stdio",
      "command": "arbor",
      "args": ["bridge"],
      "envFile": "${workspaceFolder}/.env"
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

> Note: VS Code’s MCP config uses a top-level `"servers"` key, whereas Cursor’s `.cursor/mcp.json` uses `"mcpServers"`. Make sure to use the schema appropriate for each client.

Create `.vscode/mcp.json`:

```json
{
  "servers": {
    "arbor": {
      "type": "stdio",
      "command": "arbor",
      "args": ["bridge"],
      "envFile": "${workspaceFolder}/.env"
    }
  },
  "inputs": []
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

## Setup for Claude Code (CLI)

Claude Code supports MCP server installation directly from terminal.

### Option A (recommended): add Arbor with CLI command

From your project root:

```bash
claude mcp add --transport stdio arbor -- arbor bridge
```

To share the server config with your team via repo-level `.mcp.json`, use project scope:

```bash
claude mcp add --transport stdio --scope project arbor -- arbor bridge
```

Then verify inside Claude Code:

```bash
claude mcp list
```

And in an active Claude Code session, run:

```text
/mcp
```

### Option B: commit `.mcp.json` manually

Create `.mcp.json` in repo root:

```json
{
  "mcpServers": {
    "arbor": {
      "type": "stdio",
      "command": "arbor",
      "args": ["bridge"],
      "env": {}
    }
  }
}
```

> Reference: Claude Code MCP docs — https://code.claude.com/docs/en/mcp

---

## Universal MCP Integration Kit (Beginner → Enterprise)

Arbor includes reusable templates and setup scripts so users can bootstrap MCP clients without hand-writing JSON.

### Included templates

- `templates/mcp/claude-code.project.mcp.json`
- `templates/mcp/cursor.project.mcp.json`
- `templates/mcp/vscode.project.mcp.json`
- `templates/mcp/claude-desktop.user.config.json`

### One-command bootstrap scripts

- macOS/Linux: `scripts/setup-mcp.sh`
- Windows: `scripts/setup-mcp.ps1`

Examples:

```bash
# Generate all project-scoped configs (.mcp.json, .cursor/mcp.json, .vscode/mcp.json)
./scripts/setup-mcp.sh --client all --target-dir .

# Generate only Cursor config in another repo
./scripts/setup-mcp.sh --client cursor --target-dir /path/to/repo
```

```powershell
# Generate all project-scoped configs in current directory
./scripts/setup-mcp.ps1 -Client all -TargetDir .

# Overwrite existing files
./scripts/setup-mcp.ps1 -Client vscode -TargetDir . -Force
```

### Recommended path by customer level

1. **Individual developers (fastest path)**
  - Run `claude mcp add --transport stdio --scope project arbor -- arbor bridge`
  - Or generate `.mcp.json` with setup scripts above.

2. **Teams (shared and versioned setup)**
  - Commit project-scoped MCP files (`.mcp.json`, `.cursor/mcp.json`, `.vscode/mcp.json`).
  - Keep secrets out of source; prefer `envFile` + local `.env`.

3. **Enterprise / managed environments**
  - Prefer centrally managed or programmatic registration where supported.
  - For VS Code, use organization-level AI/MCP policy controls.
  - For Cursor, use extension API registration for managed onboarding flows.

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

This guide is for Arbor releases with MCP capabilities (v1.6+). For branch/release channel policy, see [`CONTRIBUTING.md`](../.github/CONTRIBUTING.md).

---

## Competitive Notes (March 2026)

Compared with code-intel MCP competitors (for example `syke`, `flyto-indexer`, `ckb`), the strongest adoption patterns are:

1. **One-command install shown first** (especially Claude Code stdio setup)
2. **Clear capability sentence** (`impact analysis`, `dependency graph`, `build gates`)
3. **Visible directory badges** (Glama/Skills Playground) on README landing area
4. **Registry/package metadata completeness** (npm/pypi/crates metadata + README)

Arbor now includes these patterns in the root README and `arbor-mcp` README.

## Why score can still appear low

Some MCP directory scores include activity/usage signals (for example: “no recent usage”).
Those cannot be fully improved by docs alone; they rise as real installs and tool calls increase.

Practical growth levers:

- Keep install command friction near zero (`claude mcp add ...` copy-paste ready)
- Add MCP usage snippets to PR templates, docs, and release notes
- Cut regular releases so directories re-index current metadata/tooling
- Encourage users to run and keep Arbor MCP enabled in daily workflows