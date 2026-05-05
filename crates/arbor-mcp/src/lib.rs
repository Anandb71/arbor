use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

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

    fn ok_envelope(tool: &str, data: Value, node_count: usize, next_tool: &str, next_args: Value) -> Value {
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
                    "description": "Returns the direct callers of a symbol (one hop upstream). Use to answer 'what calls this function?'",
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
                    "description": "Fuzzy-searches symbol names across the graph. Use when you know part of a name but not the full ID.",
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
                    "description": "Returns all symbols and internal call edges within a single file. Use to understand a file's internal structure.",
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
                let entries: Vec<Value> = eps.iter().map(|n| json!({
                    "id": n.id,
                    "name": n.name,
                    "kind": n.kind.to_string(),
                    "file": n.file,
                    "line": n.line_start
                })).collect();
                let count = entries.len();
                let next_symbol = entries.first()
                    .and_then(|e| e["name"].as_str())
                    .unwrap_or("")
                    .to_string();
                Ok(Self::ok_envelope(
                    "list_entry_points",
                    json!({ "entry_points": entries }),
                    count,
                    "analyze_impact",
                    json!({ "node_id": next_symbol }),
                ))
            }
            "get_callers" => {
                let symbol = arguments.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
                let graph = self.graph.read().await;
                let idx = graph.get_index(symbol)
                    .or_else(|| graph.find_by_name(symbol).first().and_then(|n| graph.get_index(&n.id)));
                match idx {
                    None => Ok(Self::err_envelope("get_callers", &format!("Symbol '{}' not found", symbol))),
                    Some(idx) => {
                        let callers = graph.get_callers(idx);
                        let items: Vec<Value> = callers.iter().map(|n| json!({
                            "id": n.id,
                            "name": n.name,
                            "kind": n.kind.to_string(),
                            "file": n.file,
                            "line": n.line_start
                        })).collect();
                        let count = items.len();
                        Ok(Self::ok_envelope(
                            "get_callers",
                            json!({ "symbol": symbol, "callers": items }),
                            count,
                            if count > 0 { "analyze_impact" } else { "search_symbols" },
                            if count > 0 { json!({ "node_id": symbol }) } else { json!({ "query": symbol }) },
                        ))
                    }
                }
            }
            "get_callees" => {
                let symbol = arguments.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
                let graph = self.graph.read().await;
                let idx = graph.get_index(symbol)
                    .or_else(|| graph.find_by_name(symbol).first().and_then(|n| graph.get_index(&n.id)));
                match idx {
                    None => Ok(Self::err_envelope("get_callees", &format!("Symbol '{}' not found", symbol))),
                    Some(idx) => {
                        let callees = graph.get_callees(idx);
                        let items: Vec<Value> = callees.iter().map(|n| json!({
                            "id": n.id,
                            "name": n.name,
                            "kind": n.kind.to_string(),
                            "file": n.file,
                            "line": n.line_start
                        })).collect();
                        let count = items.len();
                        let first_callee = items.first()
                            .and_then(|e| e["name"].as_str())
                            .unwrap_or("").to_string();
                        Ok(Self::ok_envelope(
                            "get_callees",
                            json!({ "symbol": symbol, "callees": items }),
                            count,
                            if count > 0 { "get_node_detail" } else { "list_entry_points" },
                            if count > 0 { json!({ "symbol": first_callee }) } else { json!({}) },
                        ))
                    }
                }
            }
            "search_symbols" => {
                let query = arguments.get("query").and_then(|v| v.as_str()).unwrap_or("");
                let limit = arguments.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
                let graph = self.graph.read().await;
                let results = graph.search(query);
                let items: Vec<Value> = results.iter().take(limit).map(|n| json!({
                    "id": n.id,
                    "name": n.name,
                    "kind": n.kind.to_string(),
                    "file": n.file,
                    "line": n.line_start
                })).collect();
                let count = items.len();
                let first = items.first().and_then(|e| e["name"].as_str()).unwrap_or("").to_string();
                Ok(Self::ok_envelope(
                    "search_symbols",
                    json!({ "query": query, "results": items }),
                    count,
                    "get_node_detail",
                    json!({ "symbol": first }),
                ))
            }
            "get_file_graph" => {
                let file_path = arguments.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
                let graph = self.graph.read().await;
                let (nodes, edges) = graph.nodes_in_file_with_edges(file_path);
                let node_items: Vec<Value> = nodes.iter().map(|n| json!({
                    "id": n.id,
                    "name": n.name,
                    "kind": n.kind.to_string(),
                    "line": n.line_start
                })).collect();
                let edge_items: Vec<Value> = edges.iter().map(|(from, to, kind)| json!({
                    "from": from,
                    "to": to,
                    "kind": kind
                })).collect();
                let count = node_items.len();
                let highest = nodes.iter()
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
                let symbol = arguments.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
                let graph = self.graph.read().await;
                let idx = graph.get_index(symbol)
                    .or_else(|| graph.find_by_name(symbol).first().and_then(|n| graph.get_index(&n.id)));
                match idx {
                    None => Ok(Self::err_envelope("get_node_detail", &format!("Symbol '{}' not found", symbol))),
                    Some(idx) => {
                        let node = graph.get(idx).unwrap();
                        let centrality = graph.centrality(idx);
                        let callers = graph.get_callers(idx);
                        let callees = graph.get_callees(idx);
                        let is_entry = arbor_graph::HeuristicsMatcher::is_likely_entry_point(node);
                        let role = if is_entry { "entry_point" }
                            else if callers.is_empty() { "unreachable" }
                            else if callees.is_empty() { "utility" }
                            else { "internal" };
                        let next = if callers.is_empty() { "get_callees" } else { "get_callers" };
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
            _ => Err(JsonRpcError {
                code: -32601,
                message: format!("Tool not found: {}", name),
                data: None,
            }),
        }
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
        let graph: SharedGraph = Arc::new(RwLock::new(ArborGraph::new()));
        McpServer::new(graph)
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
        let result = server.call_tool(serde_json::json!({
            "name": "get_callers", "arguments": { "symbol": "nonexistent" }
        })).await;
        let val = result.unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["ok"], false);
        assert!(envelope["error"].is_string());
    }

    #[tokio::test]
    async fn test_get_callees_not_found() {
        let server = empty_server();
        let result = server.call_tool(serde_json::json!({
            "name": "get_callees", "arguments": { "symbol": "nonexistent" }
        })).await;
        let val = result.unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["ok"], false);
    }

    #[tokio::test]
    async fn test_search_symbols_returns_envelope() {
        let server = empty_server();
        let result = server.call_tool(serde_json::json!({
            "name": "search_symbols", "arguments": { "query": "main" }
        })).await;
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
        let result = server.call_tool(serde_json::json!({
            "name": "get_file_graph", "arguments": { "file_path": "src/nonexistent.rs" }
        })).await;
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
        let result = server.call_tool(serde_json::json!({
            "name": "get_node_detail", "arguments": { "symbol": "nonexistent" }
        })).await;
        let val = result.unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["ok"], false);
    }

    #[tokio::test]
    async fn test_unknown_tool_returns_error() {
        let server = empty_server();
        let result = server.call_tool(serde_json::json!({
            "name": "does_not_exist", "arguments": {}
        })).await;
        assert!(result.is_err());
    }
}
