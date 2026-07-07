# Arbor v2.4.0 — "The Agent-Native Leap" Release Notes

**Release date:** July 2026  
**Theme:** First code-graph MCP server built for MCP `2026-07-28`

---

## Highlights

### MCP 2026-07-28 Protocol Support
- Protocol version `2026-07-28` with dual-version fallback for `2025-03-26` clients
- Stateless `server/discover` endpoint
- Response caching metadata (`ttlMs`, `cacheScope`) on list/read operations
- Extensions capability map for Tasks and MCP Apps

### Tasks Extension (`io.modelcontextprotocol/tasks`)
- `tasks/get`, `tasks/update`, `tasks/cancel` for long-running operations
- Background indexing returns task handles instead of errors during cold start
- Agents poll task progress while the graph builds

### MCP Apps (SEP-1865)
- Interactive **blast-radius graph** UI (`ui://arbor/blast-radius`)
- Interactive **architecture map** UI (`ui://arbor/architecture-map`)
- `analyze_impact` and `get_architecture_overview` declare `_meta.ui` resource URIs

### Streamable HTTP Transport
```bash
arbor bridge --http --port 3333
```
- Stateless HTTP alongside stdio
- `Mcp-Method` / `Mcp-Name` header routing
- Deploy behind load balancers for remote/enterprise use

### Tool Quality
- **`get_blast_radius`** now performs real git-diff analysis (no longer a stub)
- **Pagination** on `search_symbols` and `get_map` (`offset`, `limit`, `hasMore`)
- Async tokio stdio (replaces blocking stdin loop)

### Benchmarks
- Criterion suite: `cargo bench -p arbor-graph`
- CI regression gate via `.github/workflows/benchmarks.yml`
- Updated `docs/BENCHMARKS.md` with token-savings methodology

---

## Upgrade Guide

### MCP Clients (Claude / Cursor)
No changes required for stdio transport. Existing config still works:
```bash
claude mcp add --transport stdio --scope project arbor -- arbor bridge
```

### HTTP Clients (new)
```bash
arbor bridge --http --port 3333
# POST http://127.0.0.1:3333/mcp
# Headers: Mcp-Method: tools/call, Mcp-Name: analyze_impact
```

### Protocol Version
Clients on `2025-03-26` continue to work. Clients supporting `2026-07-28` get Tasks, Apps, caching, and pagination.

---

## Launch Checklist

- [ ] Tag `v2.4.0` and publish GitHub Release
- [ ] Publish to crates.io (`cargo publish -p arbor-graph-cli`)
- [ ] Update MCP registry (`io.github.Anandb71/arbor`)
- [ ] Update Glama listing
- [ ] Record demo GIF: MCP App graph in Claude/Cursor
- [ ] Post to HN/Reddit/X timed with MCP spec RC (July 28)
- [ ] Update VS Code Marketplace extension

---

## Deferred to v2.5.0

- Parallel (rayon) indexing
- Incremental PageRank
- Unified parser pipelines
- Process-level graph daemon

See [ROADMAP_v2.4.0.md](ROADMAP_v2.4.0.md) for full planning context.
