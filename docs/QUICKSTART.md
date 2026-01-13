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

# Search for a symbol
arbor query parse_file

# Get refactoring context
arbor refactor UserService

# Explain a function's dependencies
arbor explain validate_input
```

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

## Flags

| Flag | Description |
|------|-------------|
| `--no-cache` | Force full re-index (skip cache) |
| `--follow-symlinks` | Include symlinked directories |
| `--files` | Show detailed file stats in `status` |

## Next Steps

- [Architecture Guide](./ARCHITECTURE.md)
- [Supported Languages](./ADDING_LANGUAGES.md)
- [MCP Protocol](./PROTOCOL.md)
