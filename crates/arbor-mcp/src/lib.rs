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
