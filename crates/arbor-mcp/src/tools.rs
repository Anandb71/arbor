//! MCP tool listing, dispatch, and map/context builders.

use super::*;
use arbor_graph::{
    changed_node_ids, compute_blast_radius, compute_centrality, is_minified_or_generated,
    is_test_file, shorten_signature, HeuristicsMatcher,
};
use serde_json::{json, Value};

impl McpServer {
    pub(crate) fn list_tools(&self) -> Result<Value, JsonRpcError> {
        Ok(json!({
            "tools": [
                {
                    "name": "get_logic_path",
                    "description": "Traces the call graph to find dependencies and usage of a function or class.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "start_node": { "type": "string", "description": "Name of the function or class to trace" }
                        },
                        "required": ["start_node"]
                    },
                    "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
                },
                {
                    "name": "analyze_impact",
                    "description": "Analyzes the impact (blast radius) of changing a node. Returns structured data with upstream/downstream affected nodes.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "node_id": { "type": "string", "description": "ID or name of the node to analyze" },
                            "max_depth": { "type": "integer", "description": "Maximum hop distance (default: 5, 0 = unlimited)", "default": 5 }
                        },
                        "required": ["node_id"]
                    },
                    "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false },
                    "_meta": apps::ui_meta(apps::UI_BLAST_RADIUS)
                },
                {
                    "name": "find_path",
                    "description": "Finds the shortest path between two nodes.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "start_node": { "type": "string", "description": "Name or ID of the start node" },
                            "end_node": { "type": "string", "description": "Name or ID of the end node" }
                        },
                        "required": ["start_node", "end_node"]
                    },
                    "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
                },
                {
                    "name": "get_knowledge_path",
                    "description": "Returns the actual Markdown 'logic path' with [[wiki links]] and causality explanation for knowledge Sections. The Aha! moment for Lattice users.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "start_node": { "type": "string", "description": "Starting knowledge Section (e.g. 'Core Habits')" }
                        },
                        "required": ["start_node"]
                    },
                    "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
                },
                {
                    "name": "list_entry_points",
                    "description": "Lists all detected production entry points: HTTP handlers, main functions, webhooks, background jobs, and CLI commands. Use this first to understand the execution surface of a codebase.",
                    "inputSchema": { "type": "object", "properties": {}, "required": [] },
                    "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
                },
                {
                    "name": "get_callers",
                    "description": "Returns the direct callers of a symbol (one hop upstream). Use INSTEAD of grep to find usages/references. Answers 'what calls this function?'",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "symbol": { "type": "string", "description": "Name or ID of the symbol to look up" }
                        },
                        "required": ["symbol"]
                    },
                    "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
                },
                {
                    "name": "get_callees",
                    "description": "Returns the direct callees of a symbol (one hop downstream). Use to answer 'what does this function call?'",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "symbol": { "type": "string", "description": "Name or ID of the symbol to look up" }
                        },
                        "required": ["symbol"]
                    },
                    "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
                },
                {
                    "name": "search_symbols",
                    "description": "Fuzzy-searches symbol names across the graph. Use INSTEAD of grep/rg/find to locate functions, classes, or files. Supports multi-term OR queries with '|' separator.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string", "description": "Partial or full symbol name to search for" },
                            "limit": { "type": "integer", "description": "Maximum results to return (default: 20)", "default": 20 },
                            "offset": { "type": "integer", "description": "Pagination offset (default: 0)", "default": 0 }
                        },
                        "required": ["query"]
                    },
                    "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
                },
                {
                    "name": "get_file_graph",
                    "description": "Returns all symbols and internal call edges within a single file. Use INSTEAD of reading/catting a file to understand its structure — shows what's defined and how it connects.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "file_path": { "type": "string", "description": "Relative path to the file (e.g. 'src/auth.rs')" }
                        },
                        "required": ["file_path"]
                    },
                    "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
                },
                {
                    "name": "get_node_detail",
                    "description": "Returns full detail for a single symbol: file, line range, kind, role, centrality rank. Use after search_symbols or list_entry_points to inspect a specific node.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "symbol": { "type": "string", "description": "Name or ID of the symbol" }
                        },
                        "required": ["symbol"]
                    },
                    "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
                },
                {
                    "name": "get_map",
                    "description": "Returns a ranked, token-budgeted skeleton of the codebase — the most important symbols ordered by centrality. RECOMMENDED FIRST CALL: use this instead of reading files or running find/tree to explore project structure. Entry points are marked with ★.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "tokens": { "type": "integer", "description": "Maximum token budget for output (default: 1024)", "default": 1024 },
                            "exclude_test": { "type": "boolean", "description": "Exclude test/spec/fixture files (default: true)", "default": true },
                            "focus": { "type": "string", "description": "Boost symbols in files matching this pattern (e.g. 'service', 'pipeline')" },
                            "offset": { "type": "integer", "description": "Pagination offset for entries (default: 0)", "default": 0 },
                            "limit": { "type": "integer", "description": "Max entries per page (default: 50)", "default": 50 }
                        },
                        "required": []
                    },
                    "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
                },
                {
                    "name": "get_blast_radius",
                    "description": "Analyzes the blast radius of current uncommitted git changes. Returns affected files, risk level, and architectural impact. Use this to understand how pending changes ripple through the codebase.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "base_ref": { "type": "string", "description": "Git ref for diff base (default: HEAD)", "default": "HEAD" },
                            "format": { "type": "string", "description": "Output format: json or markdown", "enum": ["json", "markdown"], "default": "json" }
                        }
                    },
                    "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
                },
                {
                    "name": "explain_symbol",
                    "description": "Returns a token-bounded architectural explanation of a symbol — its role, callers, callees, centrality rank, and significance. Ideal for understanding unfamiliar code.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "symbol": { "type": "string", "description": "Name or ID of the symbol to explain" },
                            "max_tokens": { "type": "integer", "description": "Maximum tokens for the explanation (default: 2000)", "default": 2000 }
                        },
                        "required": ["symbol"]
                    },
                    "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
                },
                {
                    "name": "audit_security",
                    "description": "Traces execution paths from a source symbol to sensitive sinks (database queries, file I/O, network calls, exec). Returns potential security-relevant paths.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "source": { "type": "string", "description": "Source symbol to trace from" },
                            "max_depth": { "type": "integer", "description": "Maximum depth (default: 8)", "default": 8 }
                        },
                        "required": ["source"]
                    },
                    "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
                },
                {
                    "name": "get_architecture_overview",
                    "description": "Returns a high-level architectural overview: top central nodes (hotspots), module boundaries, entry points, languages detected, and graph statistics. Use first to orient in an unfamiliar codebase.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "top_n": { "type": "integer", "description": "Number of top hotspots to include (default: 20)", "default": 20 }
                        }
                    },
                    "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false },
                    "_meta": apps::ui_meta(apps::UI_ARCHITECTURE_MAP)
                },
                {
                    "name": "batch_query",
                    "description": "Query multiple symbols in a single call. Returns node details for all matched symbols, reducing round-trips. Optionally includes callers/callees for each.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "symbols": { "type": "array", "items": { "type": "string" }, "description": "List of symbol names or IDs" },
                            "include_callers": { "type": "boolean", "description": "Include caller list for each symbol (default: false)", "default": false },
                            "include_callees": { "type": "boolean", "description": "Include callee list for each symbol (default: false)", "default": false }
                        },
                        "required": ["symbols"]
                    },
                    "annotations": { "readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false }
                }
            ]
        }))
    }

    pub(crate) async fn call_tool(&self, params: Value) -> Result<Value, JsonRpcError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing 'name' parameter".to_string(),
                data: None,
            })?;

        let arguments = params.get("arguments").unwrap_or(&Value::Null);

        // If the graph is empty, return a task handle instead of erroring (Tasks extension)
        if self.graph.read().await.node_count() == 0 {
            if let Some(task_resp) = self.maybe_wait_for_index(name).await {
                return Ok(task_resp);
            }
        }

        match name {
            "get_logic_path" => {
                let start_node = arguments
                    .get("start_node")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                // Trigger Spotlight so the Visualizer shows what the AI is looking at
                self.trigger_spotlight(start_node).await;

                let context = self.generate_context(start_node).await;
                Ok(json!({
                    "content": [
                        {
                            "type": "text",
                            "text": context
                        }
                    ]
                }))
            }
            "analyze_impact" => {
                let node_id = arguments
                    .get("node_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let max_depth = arguments
                    .get("max_depth")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(5) as usize;

                // Trigger Spotlight
                self.trigger_spotlight(node_id).await;

                let graph = self.graph.read().await;

                // Resolve node by name or ID
                let node_index = graph
                    .get_index(node_id)
                    .or_else(|| graph.resolve_symbol(node_id));

                match node_index {
                    Some(idx) => {
                        let analysis = graph.analyze_impact(idx, max_depth);

                        // Compute confidence and role
                        let confidence =
                            arbor_graph::ConfidenceExplanation::from_analysis(&analysis);
                        let role = arbor_graph::NodeRole::from_analysis(&analysis);

                        // Build structured response
                        let upstream: Vec<Value> = analysis
                            .upstream
                            .iter()
                            .map(|n| {
                                json!({
                                    "id": n.node_info.id,
                                    "name": n.node_info.name,
                                    "kind": n.node_info.kind,
                                    "file": n.node_info.file,
                                    "severity": n.severity.as_str(),
                                    "hop_distance": n.hop_distance,
                                    "entry_edge": n.entry_edge.to_string()
                                })
                            })
                            .collect();

                        let downstream: Vec<Value> = analysis
                            .downstream
                            .iter()
                            .map(|n| {
                                json!({
                                    "id": n.node_info.id,
                                    "name": n.node_info.name,
                                    "kind": n.node_info.kind,
                                    "file": n.node_info.file,
                                    "severity": n.severity.as_str(),
                                    "hop_distance": n.hop_distance,
                                    "entry_edge": n.entry_edge.to_string()
                                })
                            })
                            .collect();

                        Ok(json!({
                            "content": [{
                                "type": "text",
                                "text": serde_json::to_string_pretty(&json!({
                                    "target": {
                                        "id": analysis.target.id,
                                        "name": analysis.target.name,
                                        "kind": analysis.target.kind,
                                        "file": analysis.target.file
                                    },
                                    "confidence": {
                                        "level": confidence.level.to_string(),
                                        "reasons": confidence.reasons
                                    },
                                    "role": role.to_string(),
                                    "upstream": upstream,
                                    "downstream": downstream,
                                    "total_affected": analysis.total_affected,
                                    "max_depth": analysis.max_depth,
                                    "query_time_ms": analysis.query_time_ms,
                                    "edges_explained": format!(
                                        "{} upstream callers, {} downstream dependencies",
                                        analysis.upstream.len(),
                                        analysis.downstream.len()
                                    ),
                                    "sorted_by_centrality": true
                                })).unwrap_or_default()
                            }]
                        }))
                    }
                    None => Ok(json!({
                        "content": [{
                            "type": "text",
                            "text": format!("Node '{}' not found in graph", node_id)
                        }]
                    })),
                }
            }
            "find_path" => {
                let start_node = arguments
                    .get("start_node")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let end_node = arguments
                    .get("end_node")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let graph = self.graph.read().await;

                let start_idx = graph
                    .get_index(start_node)
                    .or_else(|| graph.resolve_symbol(start_node));
                let end_idx = graph
                    .get_index(end_node)
                    .or_else(|| graph.resolve_symbol(end_node));

                match (start_idx, end_idx) {
                    (Some(u), Some(v)) => {
                        if let Some(path) = graph.find_path(u, v) {
                            let path_str = path
                                .iter()
                                .map(|n| format!("`{}` ({})", n.name, n.kind))
                                .collect::<Vec<_>>()
                                .join(" -> ");
                            Ok(json!({
                                "content": [{ "type": "text", "text": format!("Found path:\n\n{}", path_str) }]
                            }))
                        } else {
                            Ok(json!({
                                "content": [{ "type": "text", "text": "No path found between these nodes." }]
                            }))
                        }
                    }
                    _ => Err(JsonRpcError {
                        code: -32602,
                        message: "Could not resolve start or end node.".to_string(),
                        data: None,
                    }),
                }
            }
            "get_knowledge_path" => {
                let start_node = arguments
                    .get("start_node")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                self.trigger_spotlight(start_node).await;

                let context = self.generate_context(start_node).await;
                let path_md = format!(
                    "**Knowledge Logic Path for `{}`** (Markdown [[links]] + causality from graph relations)\n\n{}\n\nThis provides the deterministic map your AI agents need — no more hallucinations on personal knowledge.",
                    start_node, context
                );
                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": path_md
                    }]
                }))
            }
            "list_entry_points" => {
                let graph = self.graph.read().await;
                let eps = graph.list_entry_points();
                let entries: Vec<Value> = eps
                    .iter()
                    .map(|n| {
                        json!({
                            "id": n.id,
                            "name": n.name,
                            "kind": n.kind.to_string(),
                            "file": n.file,
                            "line": n.line_start
                        })
                    })
                    .collect();
                let count = entries.len();
                let next_node_id = entries
                    .first()
                    .and_then(|e| e["id"].as_str())
                    .unwrap_or("")
                    .to_string();
                let (next_tool, next_args) = if count > 0 {
                    ("analyze_impact", json!({ "node_id": next_node_id }))
                } else {
                    ("search_symbols", json!({ "query": "" }))
                };
                Ok(Self::ok_envelope(
                    "list_entry_points",
                    json!({ "entry_points": entries }),
                    count,
                    next_tool,
                    next_args,
                ))
            }
            "get_callers" => {
                let symbol = arguments
                    .get("symbol")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let graph = self.graph.read().await;
                let resolved = graph
                    .get_index(symbol)
                    .map(|idx| (symbol.to_string(), idx))
                    .or_else(|| {
                        graph
                            .resolve_symbol(symbol)
                            .and_then(|idx| graph.get(idx).map(|n| (n.id.clone(), idx)))
                    });
                match resolved {
                    None => Ok(Self::err_envelope(
                        "get_callers",
                        &format!("Symbol '{}' not found", symbol),
                    )),
                    Some((resolved_id, idx)) => {
                        let callers = graph.get_callers(idx);
                        let items: Vec<Value> = callers
                            .iter()
                            .map(|n| {
                                json!({
                                    "id": n.id,
                                    "name": n.name,
                                    "kind": n.kind.to_string(),
                                    "file": n.file,
                                    "line": n.line_start
                                })
                            })
                            .collect();
                        let count = items.len();
                        Ok(Self::ok_envelope(
                            "get_callers",
                            json!({ "symbol": symbol, "callers": items }),
                            count,
                            if count > 0 {
                                "analyze_impact"
                            } else {
                                "search_symbols"
                            },
                            if count > 0 {
                                json!({ "node_id": resolved_id })
                            } else {
                                json!({ "query": symbol })
                            },
                        ))
                    }
                }
            }
            "get_callees" => {
                let symbol = arguments
                    .get("symbol")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let graph = self.graph.read().await;
                let resolved = graph
                    .get_index(symbol)
                    .map(|idx| (symbol.to_string(), idx))
                    .or_else(|| {
                        graph
                            .resolve_symbol(symbol)
                            .and_then(|idx| graph.get(idx).map(|n| (n.id.clone(), idx)))
                    });
                match resolved {
                    None => Ok(Self::err_envelope(
                        "get_callees",
                        &format!("Symbol '{}' not found", symbol),
                    )),
                    Some((_resolved_id, idx)) => {
                        let callees = graph.get_callees(idx);
                        let items: Vec<Value> = callees
                            .iter()
                            .map(|n| {
                                json!({
                                    "id": n.id,
                                    "name": n.name,
                                    "kind": n.kind.to_string(),
                                    "file": n.file,
                                    "line": n.line_start
                                })
                            })
                            .collect();
                        let count = items.len();
                        let first_callee_id = items
                            .first()
                            .and_then(|e| e["id"].as_str())
                            .unwrap_or("")
                            .to_string();
                        Ok(Self::ok_envelope(
                            "get_callees",
                            json!({ "symbol": symbol, "callees": items }),
                            count,
                            if count > 0 {
                                "get_node_detail"
                            } else {
                                "list_entry_points"
                            },
                            if count > 0 {
                                json!({ "symbol": first_callee_id })
                            } else {
                                json!({})
                            },
                        ))
                    }
                }
            }
            "search_symbols" => {
                let query = arguments
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let limit = arguments
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(20) as usize;
                let offset = arguments
                    .get("offset")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let graph = self.graph.read().await;
                let all_results = graph.search(query);
                let total = all_results.len();
                let page_results: Vec<_> = all_results.iter().skip(offset).take(limit).collect();
                let items: Vec<Value> = page_results
                    .iter()
                    .map(|n| {
                        json!({
                            "id": n.id,
                            "name": n.name,
                            "kind": n.kind.to_string(),
                            "file": n.file,
                            "line": n.line_start
                        })
                    })
                    .collect();
                let count = items.len();
                let first = items
                    .first()
                    .and_then(|e| e["name"].as_str())
                    .unwrap_or("")
                    .to_string();
                let has_more = offset + count < total;
                Ok(Self::ok_envelope(
                    "search_symbols",
                    json!({
                        "query": query,
                        "results": items,
                        "pagination": {
                            "offset": offset,
                            "limit": limit,
                            "total": total,
                            "hasMore": has_more,
                            "nextOffset": if has_more { json!(offset + count) } else { Value::Null }
                        }
                    }),
                    count,
                    "get_node_detail",
                    json!({ "symbol": first }),
                ))
            }
            "get_file_graph" => {
                let file_path = arguments
                    .get("file_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let graph = self.graph.read().await;
                let (nodes, edges) = graph.nodes_in_file_with_edges(file_path);
                let node_items: Vec<Value> = nodes
                    .iter()
                    .map(|n| {
                        json!({
                            "id": n.id,
                            "name": n.name,
                            "kind": n.kind.to_string(),
                            "line": n.line_start
                        })
                    })
                    .collect();
                let edge_items: Vec<Value> = edges
                    .iter()
                    .map(|(from, to, kind)| {
                        json!({
                            "from": from,
                            "to": to,
                            "kind": kind
                        })
                    })
                    .collect();
                let count = node_items.len();
                let highest = nodes
                    .iter()
                    .max_by_key(|n| n.line_end.saturating_sub(n.line_start))
                    .map(|n| n.name.clone())
                    .unwrap_or_default();
                Ok(Self::ok_envelope(
                    "get_file_graph",
                    json!({ "file": file_path, "nodes": node_items, "edges": edge_items }),
                    count,
                    "analyze_impact",
                    json!({ "node_id": highest }),
                ))
            }
            "get_node_detail" => {
                let symbol = arguments
                    .get("symbol")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let graph = self.graph.read().await;
                let idx = graph
                    .get_index(symbol)
                    .or_else(|| graph.resolve_symbol(symbol));
                match idx {
                    None => Ok(Self::err_envelope(
                        "get_node_detail",
                        &format!("Symbol '{}' not found", symbol),
                    )),
                    Some(idx) => {
                        let node = match graph.get(idx) {
                            Some(n) => n,
                            None => {
                                return Ok(Self::err_envelope(
                                    "get_node_detail",
                                    "Node index invalid",
                                ))
                            }
                        };
                        let centrality = graph.centrality(idx);
                        let callers = graph.get_callers(idx);
                        let callees = graph.get_callees(idx);
                        let is_entry = arbor_graph::HeuristicsMatcher::is_likely_entry_point(node);
                        let role = if is_entry {
                            "entry_point"
                        } else if callers.is_empty() {
                            "unreachable"
                        } else if callees.is_empty() {
                            "utility"
                        } else {
                            "internal"
                        };
                        let next = if callers.is_empty() {
                            "get_callees"
                        } else {
                            "get_callers"
                        };
                        Ok(Self::ok_envelope(
                            "get_node_detail",
                            json!({
                                "id": node.id,
                                "name": node.name,
                                "kind": node.kind.to_string(),
                                "file": node.file,
                                "line_start": node.line_start,
                                "line_end": node.line_end,
                                "signature": node.signature,
                                "centrality": centrality,
                                "role": role,
                                "caller_count": callers.len(),
                                "callee_count": callees.len(),
                                "is_entry_point": is_entry
                            }),
                            1,
                            next,
                            json!({ "symbol": symbol }),
                        ))
                    }
                }
            }
            "get_map" => {
                let token_budget = arguments
                    .get("tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1024) as usize;
                let exclude_test = arguments
                    .get("exclude_test")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let focus_pattern = arguments
                    .get("focus")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let offset = arguments
                    .get("offset")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let limit = arguments
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(50) as usize;

                let graph = self.graph.read().await;

                let has_centrality = graph.node_indexes().any(|idx| graph.centrality(idx) > 0.0);

                drop(graph);
                if !has_centrality {
                    let mut graph = self.graph.write().await;
                    let scores = compute_centrality(&graph, 20, 0.85);
                    graph.set_centrality(scores.into_map());
                }

                let graph = self.graph.read().await;
                let mut result = self.build_map(&graph, token_budget, exclude_test, focus_pattern);

                if let Some(entries) = result.get_mut("entries").and_then(|v| v.as_array()) {
                    let total = entries.len();
                    let page: Vec<Value> =
                        entries.iter().skip(offset).take(limit).cloned().collect();
                    let has_more = offset + page.len() < total;
                    if let Some(obj) = result.as_object_mut() {
                        obj.insert("entries".to_string(), json!(page));
                        obj.insert(
                            "pagination".to_string(),
                            json!({
                                "offset": offset,
                                "limit": limit,
                                "total": total,
                                "hasMore": has_more,
                                "nextOffset": if has_more { json!(offset + page.len()) } else { Value::Null }
                            }),
                        );
                    }
                }

                Ok(Self::ok_envelope(
                    "get_map",
                    result,
                    token_budget,
                    "search_symbols",
                    json!({ "query": "" }),
                ))
            }
            "get_blast_radius" => {
                let depth = arguments
                    .get("max_depth")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(5) as usize;
                let format = arguments
                    .get("format")
                    .and_then(|v| v.as_str())
                    .unwrap_or("json");

                let graph = self.graph.read().await;
                let node_count = graph.node_count();

                match git::list_changed_files(&self.project_root) {
                    Err(e) => Ok(Self::err_envelope(
                        "get_blast_radius",
                        &format!("Git diff failed: {}", e),
                    )),
                    Ok(changed_files) => {
                        if changed_files.is_empty() {
                            let data = json!({
                                "changed_files": [],
                                "blast_radius_nodes": 0,
                                "risk_level": "low",
                                "message": "No uncommitted changes detected"
                            });
                            return Ok(Self::ok_envelope(
                                "get_blast_radius",
                                data,
                                node_count,
                                "list_entry_points",
                                json!({}),
                            ));
                        }

                        let changed_ids =
                            changed_node_ids(&graph, &changed_files, &self.project_root);
                        let summary = compute_blast_radius(
                            &graph,
                            changed_files,
                            changed_ids,
                            depth,
                            &self.project_root,
                        );

                        if format == "markdown" {
                            let mut markdown = format!(
                                "## 🌳 Arbor Blast Radius Report\n\n\
                                **Risk Level:** {} | **Blast Radius:** {} nodes | **Changed Symbols:** {}\n\n\
                                ### Changed Files\n\n| File |\n|------|\n",
                                summary.risk_level,
                                summary.blast_radius_nodes,
                                summary.changed_symbols
                            );
                            for f in &summary.changed_files {
                                markdown.push_str(&format!("| `{}` |\n", f));
                            }
                            if let Some(ref diagram) = summary.mermaid_diagram {
                                markdown.push_str("\n### Impact Graph\n\n```mermaid\n");
                                markdown.push_str(diagram);
                                markdown.push_str("\n```\n");
                            }
                            markdown.push_str(&format!(
                                "\n### Impact Summary\n\n\
                                - Direct callers: {}\n\
                                - Indirect callers: {}\n\
                                - Entry points affected: {}\n\
                                - Files likely requiring updates: {}\n",
                                summary.direct_callers,
                                summary.indirect_callers,
                                summary.entrypoints_affected,
                                summary.files_likely_updates
                            ));
                            Ok(json!({
                                "content": [{ "type": "text", "text": markdown }]
                            }))
                        } else {
                            Ok(Self::ok_envelope(
                                "get_blast_radius",
                                json!({
                                    "changed_files": summary.changed_files,
                                    "changed_symbols": summary.changed_symbols,
                                    "direct_callers": summary.direct_callers,
                                    "indirect_callers": summary.indirect_callers,
                                    "entrypoints_affected": summary.entrypoints_affected,
                                    "files_likely_updates": summary.files_likely_updates,
                                    "blast_radius_nodes": summary.blast_radius_nodes,
                                    "risk_level": summary.risk_level,
                                    "mermaid_diagram": summary.mermaid_diagram
                                }),
                                node_count,
                                "analyze_impact",
                                json!({}),
                            ))
                        }
                    }
                }
            }
            "explain_symbol" => {
                let symbol = arguments
                    .get("symbol")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| JsonRpcError {
                        code: -32602,
                        message: "Missing 'symbol' parameter".to_string(),
                        data: None,
                    })?;

                self.trigger_spotlight(symbol).await;
                let graph = self.graph.read().await;

                let resolved = graph
                    .get_index(symbol)
                    .or_else(|| graph.resolve_symbol(symbol));

                match resolved {
                    None => Ok(Self::err_envelope(
                        "explain_symbol",
                        &format!("Symbol '{}' not found", symbol),
                    )),
                    Some(idx) => {
                        let node = graph.get(idx).unwrap();
                        let centrality = graph.centrality(idx);
                        let callers = graph.get_callers(idx);
                        let callees = graph.get_callees(idx);
                        let is_entry = arbor_graph::HeuristicsMatcher::is_likely_entry_point(node);

                        let role = if is_entry {
                            "entry_point"
                        } else if callers.is_empty() {
                            "unreachable"
                        } else if callees.is_empty() {
                            "utility"
                        } else {
                            "internal"
                        };

                        let markdown = format!(
                            "### 🌳 Symbol Explanation: `{}`\n\n\
                            - **Kind**: {}\n\
                            - **Location**: `{}` (Lines {}-{})\n\
                            - **Centrality Rank**: {:.4}\n\
                            - **Classified Role**: **{}**\n\n\
                            #### Callers (Direct Dependencies Upstream): {}\n\
                            {}\n\n\
                            #### Callees (Direct Dependencies Downstream): {}\n\
                            {}",
                            node.name,
                            node.kind,
                            node.file,
                            node.line_start,
                            node.line_end,
                            centrality,
                            role,
                            callers.len(),
                            callers
                                .iter()
                                .take(5)
                                .map(|n| format!("- `{}` (`{}`)", n.name, n.file))
                                .collect::<Vec<_>>()
                                .join("\n"),
                            callees.len(),
                            callees
                                .iter()
                                .take(5)
                                .map(|n| format!("- `{}` (`{}`)", n.name, n.file))
                                .collect::<Vec<_>>()
                                .join("\n")
                        );

                        Ok(json!({
                            "content": [{
                                "type": "text",
                                "text": markdown
                            }]
                        }))
                    }
                }
            }
            "audit_security" => {
                let source = arguments
                    .get("source")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| JsonRpcError {
                        code: -32602,
                        message: "Missing 'source' parameter".to_string(),
                        data: None,
                    })?;
                let max_depth = arguments
                    .get("max_depth")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(8) as usize;

                self.trigger_spotlight(source).await;
                let graph = self.graph.read().await;

                let resolved = graph
                    .get_index(source)
                    .or_else(|| graph.resolve_symbol(source));

                match resolved {
                    None => Ok(Self::err_envelope(
                        "audit_security",
                        &format!("Source symbol '{}' not found", source),
                    )),
                    Some(idx) => {
                        let dependents = graph.get_dependents(idx, max_depth);

                        let sensitive_patterns = [
                            "query",
                            "exec",
                            "eval",
                            "write",
                            "delete",
                            "connect",
                            "send",
                            "request",
                            "fetch",
                            "open",
                            "read_file",
                            "spawn",
                            "database",
                            "sql",
                            "db",
                            "auth",
                            "login",
                            "password",
                        ];

                        let mut flagged_paths = Vec::new();
                        for (dep_idx, hop_distance) in dependents {
                            let dep_node = graph.get(dep_idx).unwrap();
                            let name_lower = dep_node.name.to_lowercase();
                            if sensitive_patterns
                                .iter()
                                .any(|pat| name_lower.contains(pat))
                            {
                                flagged_paths.push(json!({
                                    "symbol": dep_node.name,
                                    "kind": dep_node.kind.to_string(),
                                    "file": dep_node.file,
                                    "line": dep_node.line_start,
                                    "hop_distance": hop_distance
                                }));
                            }
                        }

                        let count = flagged_paths.len();
                        Ok(Self::ok_envelope(
                            "audit_security",
                            json!({
                                "source": source,
                                "flagged_sinks": flagged_paths
                            }),
                            count,
                            "get_node_detail",
                            if count > 0 {
                                json!({ "symbol": flagged_paths[0]["symbol"].as_str().unwrap() })
                            } else {
                                json!({ "symbol": source })
                            },
                        ))
                    }
                }
            }
            "get_architecture_overview" => {
                let top_n = arguments
                    .get("top_n")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(20) as usize;

                let graph = self.graph.read().await;
                let node_count = graph.node_count();
                let edge_count = graph.edge_count();

                let mut nodes_with_centrality = Vec::new();
                for node_idx in graph.node_indexes() {
                    if let Some(node) = graph.get(node_idx) {
                        let centrality = graph.centrality(node_idx);
                        nodes_with_centrality.push((node, centrality));
                    }
                }

                nodes_with_centrality
                    .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

                let hotspots: Vec<Value> = nodes_with_centrality
                    .iter()
                    .take(top_n)
                    .map(|(node, centrality)| {
                        json!({
                            "id": node.id,
                            "name": node.name,
                            "kind": node.kind.to_string(),
                            "file": node.file,
                            "centrality": centrality
                        })
                    })
                    .collect();

                let entry_points = graph.list_entry_points();
                let entry_list: Vec<Value> = entry_points
                    .iter()
                    .take(10)
                    .map(|n| {
                        json!({
                            "name": n.name,
                            "kind": n.kind.to_string(),
                            "file": n.file
                        })
                    })
                    .collect();

                let mut modules = std::collections::HashSet::new();
                for node_idx in graph.node_indexes() {
                    if let Some(node) = graph.get(node_idx) {
                        let path = std::path::Path::new(&node.file);
                        if let Some(parent) = path.parent() {
                            if let Some(parent_str) = parent.to_str() {
                                if !parent_str.is_empty() {
                                    modules.insert(parent_str.to_string());
                                }
                            }
                        }
                    }
                }

                let modules_list: Vec<String> = modules.into_iter().collect();

                let data = json!({
                    "node_count": node_count,
                    "edge_count": edge_count,
                    "modules": modules_list,
                    "top_hotspots": hotspots,
                    "entry_points": entry_list
                });

                let mut next_args = json!({});
                if let Some(first_hotspot) = hotspots.first() {
                    next_args = json!({ "node_id": first_hotspot["id"].as_str().unwrap() });
                }

                Ok(Self::ok_envelope(
                    "get_architecture_overview",
                    data,
                    node_count,
                    "analyze_impact",
                    next_args,
                ))
            }
            "batch_query" => {
                let symbols = arguments
                    .get("symbols")
                    .and_then(|v| v.as_array())
                    .ok_or_else(|| JsonRpcError {
                        code: -32602,
                        message: "Missing or invalid 'symbols' parameter".to_string(),
                        data: None,
                    })?;
                let include_callers = arguments
                    .get("include_callers")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let include_callees = arguments
                    .get("include_callees")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let graph = self.graph.read().await;
                let mut results = Vec::new();

                for sym_val in symbols {
                    if let Some(sym) = sym_val.as_str() {
                        let idx = graph.get_index(sym).or_else(|| graph.resolve_symbol(sym));

                        if let Some(idx) = idx {
                            if let Some(node) = graph.get(idx) {
                                let mut detail = json!({
                                    "id": node.id,
                                    "name": node.name,
                                    "kind": node.kind.to_string(),
                                    "file": node.file,
                                    "line_start": node.line_start,
                                    "line_end": node.line_end,
                                    "centrality": graph.centrality(idx)
                                });

                                if include_callers {
                                    let callers = graph.get_callers(idx);
                                    detail["callers"] = json!(callers.iter().map(|n| {
                                        json!({ "id": n.id, "name": n.name, "kind": n.kind.to_string(), "file": n.file })
                                    }).collect::<Vec<_>>());
                                }

                                if include_callees {
                                    let callees = graph.get_callees(idx);
                                    detail["callees"] = json!(callees.iter().map(|n| {
                                        json!({ "id": n.id, "name": n.name, "kind": n.kind.to_string(), "file": n.file })
                                    }).collect::<Vec<_>>());
                                }

                                results.push(detail);
                            }
                        }
                    }
                }

                let count = results.len();
                Ok(Self::ok_envelope(
                    "batch_query",
                    json!({ "results": results }),
                    count,
                    "analyze_impact",
                    if count > 0 {
                        json!({ "node_id": results[0]["id"].as_str().unwrap() })
                    } else {
                        json!({})
                    },
                ))
            }
            _ => Err(JsonRpcError {
                code: -32601,
                message: format!("Tool not found: {}", name),
                data: None,
            }),
        }
    }

    fn build_map(
        &self,
        graph: &arbor_graph::ArborGraph,
        token_budget: usize,
        exclude_test: bool,
        focus_pattern: &str,
    ) -> Value {
        let max_per_file: usize = if token_budget <= 1024 {
            5
        } else if token_budget <= 2048 {
            8
        } else {
            12
        };

        struct ScoredNode {
            name: String,
            kind: String,
            file: String,
            line_start: u32,
            signature: Option<String>,
            score: f64,
            is_entry_point: bool,
            callers: usize,
        }

        let mut scored: Vec<ScoredNode> = Vec::new();
        for idx in graph.node_indexes() {
            let node = match graph.get(idx) {
                Some(n) => n,
                None => continue,
            };

            let kind_str = node.kind.to_string();
            if kind_str == "import" || kind_str == "export" || kind_str == "module" {
                continue;
            }

            if exclude_test && is_test_file(&node.file) {
                continue;
            }

            if is_minified_or_generated(&node.file) {
                continue;
            }

            let centrality = graph.centrality(idx);
            let is_entry = HeuristicsMatcher::is_likely_entry_point(node);
            let caller_count = graph.get_callers(idx).len();

            let kind_boost = match kind_str.as_str() {
                "class" | "interface" | "struct" => 0.1,
                "constructor" => -0.1,
                "field" | "constant" => -0.2,
                _ => 0.0,
            };
            let entry_boost = if is_entry { 0.3 } else { 0.0 };
            let focus_boost = if !focus_pattern.is_empty() && node.file.contains(focus_pattern) {
                0.3
            } else {
                0.0
            };
            let score = centrality + entry_boost + kind_boost + focus_boost;

            scored.push(ScoredNode {
                name: node.name.clone(),
                kind: kind_str,
                file: node.file.clone(),
                line_start: node.line_start,
                signature: node.signature.clone(),
                score,
                is_entry_point: is_entry,
                callers: caller_count,
            });
        }

        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let total_symbols = scored.len();

        // Group by file with per-file cap
        let mut file_order: Vec<String> = Vec::new();
        let mut file_groups: std::collections::HashMap<String, Vec<&ScoredNode>> =
            std::collections::HashMap::new();
        for node in &scored {
            let group = file_groups.entry(node.file.clone()).or_default();
            if group.len() >= max_per_file {
                continue;
            }
            if group.is_empty() {
                file_order.push(node.file.clone());
            }
            group.push(node);
        }

        // Build JSON entries within budget
        let budget_chars = token_budget * 4;
        let mut entries: Vec<Value> = Vec::new();
        let mut symbols_shown = 0;
        let mut chars_used = 0;

        for file_path in &file_order {
            let symbols = match file_groups.get(file_path) {
                Some(s) => s,
                None => continue,
            };

            let mut sym_items: Vec<Value> = Vec::new();
            for node in symbols {
                let sig_short = node
                    .signature
                    .as_deref()
                    .map(shorten_signature)
                    .unwrap_or_else(|| node.name.clone());

                let item_cost = sig_short.len() + 50;
                if chars_used + item_cost > budget_chars && symbols_shown > 0 {
                    break;
                }

                sym_items.push(json!({
                    "name": node.name,
                    "kind": node.kind,
                    "line": node.line_start,
                    "centrality": (node.score * 100.0).round() / 100.0,
                    "callers": node.callers,
                    "is_entry_point": node.is_entry_point,
                    "signature_short": sig_short,
                }));
                symbols_shown += 1;
                chars_used += item_cost;
            }

            if !sym_items.is_empty() {
                entries.push(json!({
                    "file": file_path,
                    "symbols": sym_items,
                }));
            }

            if chars_used >= budget_chars {
                break;
            }
        }

        json!({
            "schema": "arbor.map.v1",
            "token_estimate": chars_used / 4,
            "symbols_shown": symbols_shown,
            "symbols_total": total_symbols,
            "files_shown": entries.len(),
            "files_total": file_order.len(),
            "entries": entries,
        })
    }

    async fn generate_context(&self, node_start: &str) -> String {
        let graph = self.graph.read().await;

        // 1. Resolve Node
        // Prefers the most connected definition when a name is ambiguous.
        let node_idx = graph.resolve_symbol(node_start);

        let node_idx = match node_idx {
            Some(idx) => idx,
            None => {
                return format!(
                    "Node '{}' not found in the graph. Check the name or ID.",
                    node_start
                )
            }
        };

        // 2. Extract Data
        let node = graph.get(node_idx).unwrap();
        let callers = graph.get_callers(node_idx);
        let callees = graph.get_callees(node_idx);
        let centrality = graph.centrality(node_idx);

        // 3. Format Output (The "Architectural Brief" with Markdown Tables)
        let mut brief = String::new();

        brief.push_str(&format!("# Architectural Brief: `{}`\n\n", node.name));
        brief.push_str("| Property | Value |\n");
        brief.push_str("|----------|-------|\n");
        brief.push_str(&format!("| **Type** | {} |\n", node.kind));
        brief.push_str(&format!("| **File** | `{}` |\n", node.file));
        brief.push_str(&format!("| **Impact Level** | {:.2} |\n", centrality));
        if let Some(sig) = &node.signature {
            brief.push_str(&format!("| **Signature** | `{}` |\n", sig));
        }

        // Dependencies Table
        brief.push_str("\n## Dependencies (Callees)\n\n");
        if callees.is_empty() {
            brief.push_str("*None - This is a leaf node.*\n");
        } else {
            brief.push_str("| Symbol | Type | Impact | File |\n");
            brief.push_str("|--------|------|--------|------|\n");
            for callee in callees {
                let callee_idx = graph.get_index(&callee.id);
                let impact = callee_idx.map(|idx| graph.centrality(idx)).unwrap_or(0.0);
                brief.push_str(&format!(
                    "| `{}` | {} | {:.2} | `{}` |\n",
                    callee.name, callee.kind, impact, callee.file
                ));
            }
        }

        // Usage Table
        brief.push_str("\n## Usage (Callers)\n\n");
        if callers.is_empty() {
            brief.push_str("*None - Potential entry point or dead code.*\n");
        } else {
            brief.push_str("| Symbol | Type | Impact | File |\n");
            brief.push_str("|--------|------|--------|------|\n");
            for caller in callers {
                let caller_idx = graph.get_index(&caller.id);
                let impact = caller_idx.map(|idx| graph.centrality(idx)).unwrap_or(0.0);
                brief.push_str(&format!(
                    "| `{}` | {} | {:.2} | `{}` |\n",
                    caller.name, caller.kind, impact, caller.file
                ));
            }
        }

        brief
    }
}
