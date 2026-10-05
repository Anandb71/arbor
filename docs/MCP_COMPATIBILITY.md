# MCP compatibility

Tested protocol/version behavior of `arbor bridge` (the MCP server). Every row
is exercised by `crates/arbor-mcp/tests/transport.rs` — the harness drives the
real stdio JSON-RPC loop over a duplex channel, so the matrix is a build
artifact, not prose.

## Protocol versions

| Client declares | Server response | Test |
|---|---|---|
| `2026-07-28` (current) | `protocolVersion: 2026-07-28`, full capabilities incl. extensions | `initialize_negotiates_both_protocol_versions` |
| `2025-03-26` (legacy) | `protocolVersion: 2025-03-26`, legacy capability set (no extensions) | same |
| Any dated `2026-*` or later (e.g. `2030-06-01`) | `protocolVersion: 2026-07-28` — newer clients get the newest the server speaks, never a downgrade to legacy | `initialize_future_version_negotiates_latest_not_legacy` |
| Malformed / non-`YYYY-*` (e.g. `banana`) | `-32602` naming the supported versions — rejected explicitly, never silently mapped | `initialize_rejects_a_malformed_version` |
| Omitted | legacy defaults | — |

## Extensions (MCP 2026-07-28)

| Extension | Status |
|---|---|
| `io.modelcontextprotocol/tasks` | Implemented — `tasks/get`, `tasks/update`, `tasks/cancel` |
| `io.modelcontextprotocol/apps` | Implemented — `ui://arbor/*` resources via `resources/list`/`resources/read` |

Extensions are opt-in: when the client sends `capabilities.extensions`, the
server advertises only the intersection. Declaring an extension Arbor does not
implement changes nothing — it is absent from the response rather than
accepted. (`extensions_are_narrowed_to_what_the_client_declared`,
`every_advertised_extension_has_a_real_method`.)

## Message behavior

| Input | Behavior | Test |
|---|---|---|
| Unknown method | `-32601` | `unknown_method_and_tool_return_errors_not_crashes` |
| Unknown tool name | tool-level error, connection stays live | same |
| Unparseable line (stdio) | `-32700` JSON-RPC response on stdout; log on stderr | `malformed_input_gets_a_parse_error_response` |
| Notification (no `id`) | no response bytes | `notifications_produce_no_response` |
| `tasks/cancel` on an unknown task | handled, not `-32601` | `every_advertised_extension_has_a_real_method` |
| Client disconnect / EOF | server exits cleanly | `client_disconnect_ends_the_server_cleanly` |
| `resources/list` → `resources/read` | `ui://arbor/*` contents round-trip | `resources_list_and_read_round_trip` |
| `_meta.io.modelcontextprotocol/protocolVersion` | namespaced key resolves the per-request protocol (flat keys kept as fallback) | `namespaced_protocol_meta_resolves_on_stateless_requests` |
| node paths in tool output | project-relative (`src/lib.rs`), never absolute | `tool_responses_carry_project_relative_paths` |

## Transports

| Transport | Shape | Notes |
|---|---|---|
| stdio (`arbor bridge .`) | newline-delimited JSON-RPC; stdout carries protocol only | negotiation persists for the process's single client |
| HTTP (`arbor bridge --http --port 3333`) | POST `/mcp`; Streamable-shaped replies; loopback Host/Origin and `application/json` enforced | stateless — negotiation lives in `_meta`, never shared between connections |

## Tool surface

`tools/list` exposes the sixteen documented tools; `search_symbols` and
`get_map` paginate (`offset`, `limit`, `hasMore`). Client-supplied `limit`,
`max_depth`, `top_n` and `tokens` are capped (500 / 32 / 500 / 64k).

## Client flows verified

- `templates/mcp/*.json` are structurally validated in CI
  (`mcp-integration-validate.yml`).
- Real client binaries (Claude Code, Cursor, VS Code) are not in the test
  matrix yet — their configs are checked, their protocol behavior is covered
  by the transport tests above.
