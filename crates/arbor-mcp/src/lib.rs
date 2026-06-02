use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

use arbor_graph::{compute_centrality, HeuristicsMatcher};
use arbor_server::{SharedGraph, SyncServerHandle};

#[derive(Serialize, Deserialize, Debug)]
struct JsonRpcRequest {
    jsonrpc: String,
    method: String,
    params: Option<Value>,
    id: Option<Value>,
}

#[derive(Serialize, Deserialize, Debug)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
    id: Option<Value>,
}

#[derive(Serialize, Deserialize, Debug)]
struct JsonRpcError {
    code: i32,
    message: String,
    data: Option<Value>,
}

pub struct McpServer {
    graph: SharedGraph,
    spotlight_handle: Option<SyncServerHandle>,
}

impl McpServer {
    pub fn new(graph: SharedGraph) -> Self {
        Self {
            graph,
            spotlight_handle: None,
        }
    }

    /// Creates an MCP server with spotlight capability.
    pub fn with_spotlight(graph: SharedGraph, handle: SyncServerHandle) -> Self {
        Self {
            graph,
            spotlight_handle: Some(handle),
        }
    }

    /// Triggers a spotlight on the visualizer for the given node.
    async fn trigger_spotlight(&self, node_name: &str) {
        if let Some(handle) = &self.spotlight_handle {
            let graph = self.graph.read().await;

            // Find the node by name or ID
            let node = if let Some(idx) = graph.get_index(node_name) {
                graph.get(idx)
            } else {
                let candidates = graph.find_by_name(node_name);
                candidates.into_iter().next()
            };

            if let Some(node) = node {
                handle.spotlight_node(&node.id, &node.file, node.line_start);
                eprintln!("Spotlight: {} in {}", node.name, node.file);
            }
        }
    }

    pub async fn run_stdio(&self) -> Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout();

        // Use blocking iterator for simplicity on stdin with lines
        // In a real async CLI, we might use tokio::io::stdin
        let lines = stdin.lock().lines();

        for line in lines {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            // Parse request
            let req: JsonRpcRequest = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Failed to parse input: {}", e);
                    continue;
                }
            };

            // Handle method
            if let Some(response) = self.handle_request(req).await {
                // Serialize and write
                let json = serde_json::to_string(&response)?;
                writeln!(stdout, "{}", json)?;
                stdout.flush()?;
            }
        }
        Ok(())
    }

    async fn handle_request(&self, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
        let id = req.id.clone();

        // Basic list_tools and call_tool implementation
        let result = match req.method.as_str() {
            "initialize" => Ok(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {},
                    "resources": {},
                    "streaming": false,
                    "pagination": false,
                    "json": true
                },
                "serverInfo": {
                    "name": "arbor-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
            "notifications/initialized" => Ok(json!({})),
            "tools/list" => self.list_tools(),
            "tools/call" => self.call_tool(req.params.unwrap_or(Value::Null)).await,
            "resources/list" => Ok(json!({ "resources": [] })),
            method => Err(JsonRpcError {
                code: -32601,
                message: format!("Method not found: {}", method),
                data: None,
            }),
        };

        id.as_ref()?;

        Some(match result {
            Ok(val) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(val),
                error: None,
                id,
            },
            Err(err) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(err),
                id,
            },
        })
    }

    fn ok_envelope(
        tool: &str,
        data: Value,
        node_count: usize,
        next_tool: &str,
        next_args: Value,
    ) -> Value {
        json!({
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(&json!({
                    "ok": true,
                    "tool": tool,
                    "arbor_version": env!("CARGO_PKG_VERSION"),
                    "data": data,
                    "meta": {
                        "node_count": node_count,
                        "suggested_next_tool": next_tool,
                        "suggested_next_args": next_args
                    }
                })).unwrap_or_default()
            }]
        })
    }

    fn err_envelope(tool: &str, message: &str) -> Value {
        json!({
            "content": [{
                "type": "text",
                "text": serde_json::to_string_pretty(&json!({
                    "ok": false,
                    "tool": tool,
                    "arbor_version": env!("CARGO_PKG_VERSION"),
                    "error": message
                })).unwrap_or_default()
            }]
        })
    }

    fn list_tools(&self) -> Result<Value, JsonRpcError> {
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
                    }
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
                    }
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
                    }
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
                    }
                },
                {
                    "name": "list_entry_points",
                    "description": "Lists all detected production entry points: HTTP handlers, main functions, webhooks, background jobs, and CLI commands. Use this first to understand the execution surface of a codebase.",
                    "inputSchema": { "type": "object", "properties": {}, "required": [] }
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
                    }
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
                    }
                },
                {
                    "name": "search_symbols",
                    "description": "Fuzzy-searches symbol names across the graph. Use INSTEAD of grep/rg/find to locate functions, classes, or files. Supports multi-term OR queries with '|' separator.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "query": { "type": "string", "description": "Partial or full symbol name to search for" },
                            "limit": { "type": "integer", "description": "Maximum results to return (default: 20)", "default": 20 }
                        },
                        "required": ["query"]
                    }
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
                    }
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
                    }
                },
                {
                    "name": "get_map",
                    "description": "Returns a ranked, token-budgeted skeleton of the codebase — the most important symbols ordered by centrality. RECOMMENDED FIRST CALL: use this instead of reading files or running find/tree to explore project structure. Entry points are marked with ★.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "tokens": { "type": "integer", "description": "Maximum token budget for output (default: 1024)", "default": 1024 },
                            "exclude_test": { "type": "boolean", "description": "Exclude test/spec/fixture files (default: true)", "default": true },
                            "focus": { "type": "string", "description": "Boost symbols in files matching this pattern (e.g. 'service', 'pipeline')" }
                        },
                        "required": []
                    }
                }
            ]
        }))
    }

    async fn call_tool(&self, params: Value) -> Result<Value, JsonRpcError> {
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing 'name' parameter".to_string(),
                data: None,
            })?;

        let arguments = params.get("arguments").unwrap_or(&Value::Null);

        // If the graph is empty, the background index hasn't finished yet
        if self.graph.read().await.node_count() == 0 {
            return Ok(Self::err_envelope(
                name,
                "Arbor is still indexing the project. Please retry in a few seconds.",
            ));
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
                let node_index = graph.get_index(node_id).or_else(|| {
                    graph
                        .find_by_name(node_id)
                        .first()
                        .and_then(|n| graph.get_index(&n.id))
                });

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

                let start_idx = graph.get_index(start_node).or_else(|| {
                    graph
                        .find_by_name(start_node)
                        .first()
                        .and_then(|n| graph.get_index(&n.id))
                });
                let end_idx = graph.get_index(end_node).or_else(|| {
                    graph
                        .find_by_name(end_node)
                        .first()
                        .and_then(|n| graph.get_index(&n.id))
                });

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
                            .find_by_name(symbol)
                            .first()
                            .and_then(|n| graph.get_index(&n.id).map(|idx| (n.id.clone(), idx)))
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
                            .find_by_name(symbol)
                            .first()
                            .and_then(|n| graph.get_index(&n.id).map(|idx| (n.id.clone(), idx)))
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
                let graph = self.graph.read().await;
                let results = graph.search(query);
                let items: Vec<Value> = results
                    .iter()
                    .take(limit)
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
                Ok(Self::ok_envelope(
                    "search_symbols",
                    json!({ "query": query, "results": items }),
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
                let idx = graph.get_index(symbol).or_else(|| {
                    graph
                        .find_by_name(symbol)
                        .first()
                        .and_then(|n| graph.get_index(&n.id))
                });
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

                let graph = self.graph.read().await;

                // Check if centrality is computed
                let has_centrality = graph.node_indexes().any(|idx| graph.centrality(idx) > 0.0);

                // If no centrality, we need a mutable graph — drop read, acquire write
                drop(graph);
                if !has_centrality {
                    let mut graph = self.graph.write().await;
                    let scores = compute_centrality(&graph, 20, 0.85);
                    graph.set_centrality(scores.into_map());
                }

                let graph = self.graph.read().await;
                let result = self.build_map(&graph, token_budget, exclude_test, focus_pattern);

                Ok(Self::ok_envelope(
                    "get_map",
                    result,
                    token_budget,
                    "search_symbols",
                    json!({ "query": "" }),
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

            if exclude_test && self.is_test_file(&node.file) {
                continue;
            }

            if Self::is_minified_or_generated(&node.file) {
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
                    .map(|s| self.shorten_signature(s))
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

    fn is_test_file(&self, file_path: &str) -> bool {
        let lower = file_path.to_lowercase();
        lower.contains("/test")
            || lower.contains("/spec")
            || lower.contains("/fixture")
            || lower.contains("/mock")
            || lower.contains("__tests__")
            || lower.contains(".test.")
            || lower.contains(".spec.")
            || lower.contains("_test.")
    }

    fn is_minified_or_generated(file_path: &str) -> bool {
        let lower = file_path.to_lowercase();
        lower.ends_with(".min.js")
            || lower.ends_with(".min.css")
            || lower.contains(".chunk.")
            || lower.contains(".bundle.")
            || lower.contains("/dist/")
            || lower.contains("/build/")
            || lower.contains("/resources/monitor/")
            || lower.contains("/resources/static/")
            || lower.contains("/generated/")
            || {
                let filename = lower.rsplit('/').next().unwrap_or("");
                let parts: Vec<&str> = filename.split('.').collect();
                parts.len() >= 3
                    && parts[1].len() >= 8
                    && parts[1].chars().all(|c| c.is_ascii_hexdigit())
            }
    }

    fn shorten_signature(&self, sig: &str) -> String {
        let sig = sig.trim();
        let paren_start = match sig.find('(') {
            Some(i) => i,
            None => {
                if sig.len() <= 80 {
                    return sig.to_string();
                } else {
                    return format!("{}...", &sig[..77]);
                }
            }
        };

        let before_paren = &sig[..paren_start];
        let name = before_paren
            .split_whitespace()
            .last()
            .unwrap_or(before_paren)
            .trim();

        let paren_end = match sig.rfind(')') {
            Some(i) => i,
            None => return format!("{}(...)", name),
        };

        let params_str = &sig[paren_start + 1..paren_end];
        let param_names = self.extract_param_names(params_str);

        let result = if param_names.is_empty() {
            format!("{}()", name)
        } else {
            format!("{}({})", name, param_names.join(", "))
        };

        if result.len() > 80 {
            format!("{}(...)", name)
        } else {
            result
        }
    }

    fn extract_param_names<'a>(&self, params_str: &'a str) -> Vec<&'a str> {
        if params_str.trim().is_empty() {
            return Vec::new();
        }

        let mut names = Vec::new();
        let mut depth: i32 = 0;
        let mut start = 0;

        let bytes = params_str.as_bytes();
        for i in 0..bytes.len() {
            match bytes[i] {
                b'<' | b'(' => depth += 1,
                b'>' | b')' => depth -= 1,
                b',' if depth == 0 => {
                    if let Some(name) = Self::last_word_of_param(&params_str[start..i]) {
                        names.push(name);
                    }
                    start = i + 1;
                }
                _ => {}
            }
        }
        if let Some(name) = Self::last_word_of_param(&params_str[start..]) {
            names.push(name);
        }

        names
    }

    fn last_word_of_param(param: &str) -> Option<&str> {
        let trimmed = param.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(colon_pos) = trimmed.find(':') {
            let before_colon = trimmed[..colon_pos].trim();
            return before_colon.split_whitespace().last();
        }
        trimmed.split_whitespace().last()
    }

    async fn generate_context(&self, node_start: &str) -> String {
        let graph = self.graph.read().await;

        // 1. Resolve Node
        let node_idx = if let Some(idx) = graph.get_index(node_start) {
            Some(idx)
        } else {
            // Try by name
            let candidates = graph.find_by_name(node_start);
            if let Some(first) = candidates.first() {
                graph.get_index(&first.id)
            } else {
                None
            }
        };

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

#[cfg(test)]
mod tool_tests {
    use super::*;
    use arbor_graph::ArborGraph;
    use arbor_server::SharedGraph;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn empty_server() -> McpServer {
        let mut graph = ArborGraph::new();
        // Add a dummy node so the "still indexing" guard passes
        let node = arbor_core::CodeNode::new("_dummy", "_dummy", arbor_core::NodeKind::Function, "_dummy.rs");
        graph.add_node(node);
        let shared: SharedGraph = Arc::new(RwLock::new(graph));
        McpServer::new(shared)
    }

    #[tokio::test]
    async fn test_list_entry_points_tool_returns_envelope() {
        let server = empty_server();
        let result = server
            .call_tool(serde_json::json!({ "name": "list_entry_points", "arguments": {} }))
            .await;
        assert!(result.is_ok());
        let val = result.unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["ok"], true);
        assert_eq!(envelope["tool"], "list_entry_points");
        assert!(envelope["data"]["entry_points"].is_array());
        assert!(envelope["meta"]["suggested_next_tool"].is_string());
    }

    #[tokio::test]
    async fn test_get_callers_not_found() {
        let server = empty_server();
        let result = server
            .call_tool(serde_json::json!({
                "name": "get_callers", "arguments": { "symbol": "nonexistent" }
            }))
            .await;
        let val = result.unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["ok"], false);
        assert!(envelope["error"].is_string());
    }

    #[tokio::test]
    async fn test_get_callees_not_found() {
        let server = empty_server();
        let result = server
            .call_tool(serde_json::json!({
                "name": "get_callees", "arguments": { "symbol": "nonexistent" }
            }))
            .await;
        let val = result.unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["ok"], false);
    }

    #[tokio::test]
    async fn test_search_symbols_returns_envelope() {
        let server = empty_server();
        let result = server
            .call_tool(serde_json::json!({
                "name": "search_symbols", "arguments": { "query": "main" }
            }))
            .await;
        let val = result.unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["ok"], true);
        assert!(envelope["data"]["results"].is_array());
        assert!(envelope["meta"]["suggested_next_tool"].is_string());
    }

    #[tokio::test]
    async fn test_get_file_graph_returns_envelope() {
        let server = empty_server();
        let result = server
            .call_tool(serde_json::json!({
                "name": "get_file_graph", "arguments": { "file_path": "src/nonexistent.rs" }
            }))
            .await;
        let val = result.unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["ok"], true);
        assert!(envelope["data"]["nodes"].is_array());
        assert!(envelope["data"]["edges"].is_array());
    }

    #[tokio::test]
    async fn test_get_node_detail_not_found() {
        let server = empty_server();
        let result = server
            .call_tool(serde_json::json!({
                "name": "get_node_detail", "arguments": { "symbol": "nonexistent" }
            }))
            .await;
        let val = result.unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["ok"], false);
    }

    #[tokio::test]
    async fn test_unknown_tool_returns_error() {
        let server = empty_server();
        let result = server
            .call_tool(serde_json::json!({
                "name": "does_not_exist", "arguments": {}
            }))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_map_returns_envelope() {
        let server = empty_server();
        let result = server
            .call_tool(serde_json::json!({
                "name": "get_map", "arguments": { "tokens": 1024 }
            }))
            .await;
        assert!(result.is_ok());
        let val = result.unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["ok"], true);
        assert_eq!(envelope["tool"], "get_map");
        assert_eq!(envelope["data"]["schema"], "arbor.map.v1");
        assert!(envelope["data"]["symbols_total"].is_number());
        assert!(envelope["data"]["entries"].is_array());
    }

    #[tokio::test]
    async fn test_get_map_with_populated_graph() {
        use arbor_core::{CodeNode, NodeKind};

        let graph = ArborGraph::new();
        let shared: SharedGraph = Arc::new(RwLock::new(graph));

        // Add some nodes
        {
            let mut g = shared.write().await;
            let mut n1 = CodeNode::new("main", "main", NodeKind::Function, "src/main.rs");
            n1.line_start = 1;
            n1.line_end = 10;
            let mut n2 = CodeNode::new("helper", "helper", NodeKind::Function, "src/lib.rs");
            n2.line_start = 5;
            n2.line_end = 15;
            n2.signature = Some("fn helper(x: i32) -> i32".to_string());
            let idx1 = g.add_node(n1);
            let idx2 = g.add_node(n2);
            g.add_edge(
                idx1,
                idx2,
                arbor_graph::Edge::new(arbor_graph::EdgeKind::Calls),
            );
        }

        let server = McpServer::new(shared);
        let result = server
            .call_tool(serde_json::json!({
                "name": "get_map", "arguments": { "tokens": 1024, "exclude_test": false }
            }))
            .await;
        assert!(result.is_ok());
        let val = result.unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["data"]["symbols_total"], 2);
        assert!(envelope["data"]["symbols_shown"].as_u64().unwrap() >= 2);

        let entries = envelope["data"]["entries"].as_array().unwrap();
        assert!(!entries.is_empty());
    }
}
