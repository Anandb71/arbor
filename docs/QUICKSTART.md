# Arbor Quickstart

Get AI-ready code context in 5 minutes.

## Install

**No-build install (recommended):**

```bash
curl -fsSL https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.sh | bash
```

**Windows PowerShell:**

```powershell
irm https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.ps1 | iex
```

**Cargo alternative:**

```bash
cargo install arbor-graph-cli
```

> **Note:** The GUI is included in the binary. Run `arbor gui` after installation.
> Need version pinning/manual assets? See [INSTALL.md](./INSTALL.md).

## Initialize (Optional)

```bash
cd your-project
arbor init
```

This creates `.arbor/` with default configuration.

> Prefer a one-shot start? Use `arbor setup` to initialize and index in a single command.

## Index

```bash
arbor index
```

Parses your codebase and builds a relationship graph. Subsequent runs use caching for faster updates.

```bash
# Fast refresh during active refactors
arbor index --changed-only
```

> If `.arbor/` doesn't exist, Arbor now auto-creates it on first index/query/refactor/explain.

## Query

```bash
# Show project stats
arbor status

# List all indexed files
arbor status --files

# Search for a symbol
arbor query parse_file

# Search in a different path
arbor query parse_file ../another-project

# Get refactoring context
arbor refactor UserService

# Explain a function's dependencies
arbor explain validate_input

# Preview impact for current git diff
arbor diff

# CI safety gate (fails on risky blast radius)
arbor check --max-blast-radius 30

# Jump directly to a symbol in your editor
arbor open parse_file
```

## Use the GUI

```bash
arbor gui
```

Opens the native graphical interface for impact analysis:
- Enter a symbol name
- Click "Analyze" to see callers, dependencies, and confidence
- File paths are hidden by default for privacy (click to reveal)
- Copy results as Markdown for PRs

![Arbor GUI](gui_screenshot.png)

## Watch Mode

Auto-refresh the index when files change:

```bash
arbor watch
```

Great for development workflows where you want continuous indexing.

## Generate PR Summaries

```bash
arbor pr-summary parse_file,validate_input
```

Generates a Markdown summary of impact for multiple changed symbols.

## Use with Cursor

1. Add to `.cursor/mcp.json`:
```json
{
  "servers": {
    "arbor": {
      "command": "arbor",
      "args": ["bridge"]
    }
  }
}
```

2. Restart Cursor.

3. Ask questions like:
   - "What depends on `UserService`?"
   - "What does `parse_file` call?"
   - "Show me the context for refactoring `validate`"

## CLI Flags

| Flag | Description |
|------|-------------|
| `--no-cache` | Force full re-index (skip cache) |
| `--follow-symlinks` | Include symlinked directories |
| `--files` | Show detailed file stats in `status` |
| `--depth N` | Set impact analysis depth (default: 5) |
| `--why` | Show detailed reasoning for each affected node |
| `--json` | Output as JSON instead of formatted text |

## Health Check

```bash
arbor doctor
```

Runs environment diagnostics (ports, workspace layout, visualizer and extension presence).

## Next Steps

- [Roadmap](./ROADMAP.md) — See what's coming
- [Architecture Guide](./ARCHITECTURE.md)
- [Supported Languages](./ADDING_LANGUAGES.md)
- [MCP Protocol](./PROTOCOL.md)
- [MCP Integration](./MCP_INTEGRATION.md)
- [Release Notes (v1.6)](./RELEASE_NOTES_v1.6.0.md)
- [Glama MCP Directory Listing](https://glama.ai/mcp/servers/@Anandb71/arbor)
