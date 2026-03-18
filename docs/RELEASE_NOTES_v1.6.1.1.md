# Arbor v1.6.1.1 — Maintenance Update (2026-03-18)

This is a maintenance release focused on reliability, release hygiene, and ecosystem alignment.

## What’s Included

### Release + workflow maintenance

- Finalized the `v1.6.1.1` maintenance cut and aligned release messaging in project docs.
- Updated Cargo workspace package version to `1.6.1` (SemVer-compliant crate line).
- Updated MCP server advertised version metadata to `1.6.1.1` for client-visible release tracking.

### MCP integration documentation refresh

- Added VS Code MCP setup (`.vscode/mcp.json`) guidance.
- Clarified trust/approval expectations when registering workspace MCP servers.
- Corrected contributor policy reference path.

### Ecosystem alignment snapshot (as of 2026-03-18)

- Rust stable line includes `1.94.0` (release announcements).
- tree-sitter latest observed release: `v0.26.7`.
- MCP ecosystem support continues to broaden across editors/clients.

## Notes

- No breaking CLI/API behavior changes are introduced in this maintenance update.
- For upcoming feature work, continue development on `main`; for maintenance-only fixes, use `release/v1.6`.
