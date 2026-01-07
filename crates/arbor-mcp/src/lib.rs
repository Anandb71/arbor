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
                handle.spotlight_node(&node.id, &node.file, node.line_start as u32);
                eprintln!("🔦 Spotlight: {} in {}", node.name, node.file);
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
                    "resources": {}
                },
                "serverInfo": {
                    "name": "arbor-mcp",
                    "version": "0.1.0"
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

        if id.is_none() {
            return None;
        }

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
                    "description": "Analyzes the impact (blast radius) of changing a specific node.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "node_id": { "type": "string", "description": "ID or name of the node to analyze" }
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

                // Trigger Spotlight
                self.trigger_spotlight(node_id).await;

                let context = self.generate_context(node_id).await;
                Ok(json!({
                    "content": [
                        {
                            "type": "text",
                            "text": format!("Impact analysis for {}:\n\n{}", node_id, context)
                        }
                    ]
                }))
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
                        .map(|n| graph.get_index(&n.id).unwrap())
                });
                let end_idx = graph.get_index(end_node).or_else(|| {
                    graph
                        .find_by_name(end_node)
                        .first()
                        .map(|n| graph.get_index(&n.id).unwrap())
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
        brief.push_str(&format!("| Property | Value |\n"));
        brief.push_str(&format!("|----------|-------|\n"));
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
