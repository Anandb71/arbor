# Arbor MCP Templates

These templates are versioned examples for supported clients.

## Files

- `claude-code.project.mcp.json` → project-scoped `.mcp.json` (Claude Code)
- `cursor.project.mcp.json` → `.cursor/mcp.json` (Cursor)
- `vscode.project.mcp.json` → `.vscode/mcp.json` (VS Code)
- `claude-desktop.user.config.json` → user config example for Claude Desktop

## Why multiple formats?

MCP hosts do not all use identical config roots:

- VS Code uses top-level `servers`
- Cursor and Claude-family configs commonly use top-level `mcpServers`

Use `scripts/setup-mcp.sh` or `scripts/setup-mcp.ps1` to generate project configs automatically.
