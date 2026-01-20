# Arbor Quickstart

Get AI-ready code context in 5 minutes.

## Install

```bash
cargo install arbor
```

## Initialize

```bash
cd your-project
arbor init
```

This creates `.arbor/` with default configuration.

## Index

```bash
arbor index
```

Parses your codebase and builds a relationship graph. Subsequent runs use caching for faster updates.

## Query

```bash
# Show project stats
arbor status

# List all indexed files
arbor status --files

# Search for a symbol
arbor query parse_file

# Get refactoring context
arbor refactor UserService

# Explain a function's dependencies
arbor explain validate_input
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

## Next Steps

- [Roadmap](./ROADMAP.md) — See what's coming
- [Architecture Guide](./ARCHITECTURE.md)
- [Supported Languages](./ADDING_LANGUAGES.md)
- [MCP Protocol](./PROTOCOL.md)
