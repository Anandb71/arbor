use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

use arbor_server::{SharedGraph, SyncServerHandle};

mod apps;
pub mod git;
mod http;
mod protocol;
mod tasks;
mod tools;

pub use git::{is_git_repo, list_changed_files};
pub use http::run_http_server;
use protocol::{
    discover_response, legacy_capabilities, parse_request_meta, resolve_protocol_version,
    server_capabilities, with_cache_meta, DEFAULT_TTL_MS, PROTOCOL_VERSION_LATEST,
    PROTOCOL_VERSION_LEGACY,
};
use tasks::TaskManager;

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
    project_root: PathBuf,
    tasks: Arc<TaskManager>,
    negotiated_protocol: Arc<tokio::sync::RwLock<Option<String>>>,
}

impl McpServer {
    pub fn new(graph: SharedGraph) -> Self {
        Self {
            graph,
            spotlight_handle: None,
            project_root: PathBuf::from("."),
            tasks: Arc::new(TaskManager::new()),
            negotiated_protocol: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Creates an MCP server with project root for git-diff blast radius.
    pub fn with_project(graph: SharedGraph, project_root: PathBuf) -> Self {
        Self {
            graph,
            spotlight_handle: None,
            project_root,
            tasks: Arc::new(TaskManager::new()),
            negotiated_protocol: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Creates an MCP server with spotlight capability.
    pub fn with_spotlight(graph: SharedGraph, handle: SyncServerHandle) -> Self {
        Self {
            graph,
            spotlight_handle: Some(handle),
            project_root: PathBuf::from("."),
            tasks: Arc::new(TaskManager::new()),
            negotiated_protocol: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    /// Creates an MCP server with spotlight and project root.
    pub fn with_spotlight_and_project(
        graph: SharedGraph,
        handle: SyncServerHandle,
        project_root: PathBuf,
    ) -> Self {
        Self {
            graph,
            spotlight_handle: Some(handle),
            project_root,
            tasks: Arc::new(TaskManager::new()),
            negotiated_protocol: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    pub fn task_manager(&self) -> Arc<TaskManager> {
        self.tasks.clone()
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
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let mut reader = BufReader::new(stdin);
        let mut line = String::new();

        loop {
            line.clear();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                break;
            }
            if line.trim().is_empty() {
                continue;
            }

            let req: JsonRpcRequest = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("Failed to parse input: {}", e);
                    continue;
                }
            };

            if let Some(response) = self.handle_request(req).await {
                let json = serde_json::to_string(&response)?;
                stdout.write_all(json.as_bytes()).await?;
                stdout.write_all(b"\n").await?;
                stdout.flush().await?;
            }
        }
        Ok(())
    }

    /// Handle a raw JSON-RPC body over HTTP transport.
    pub async fn handle_http_body(&self, body: &str) -> String {
        let req: JsonRpcRequest = match serde_json::from_str(body) {
            Ok(r) => r,
            Err(e) => {
                return serde_json::to_string(&JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {}", e),
                        data: None,
                    }),
                    id: None,
                })
                .unwrap_or_default();
            }
        };

        match self.handle_request(req).await {
            Some(resp) => serde_json::to_string(&resp).unwrap_or_default(),
            None => "{}".to_string(),
        }
    }

    /// If graph is empty, spawn a background wait task and return task handle.
    async fn maybe_wait_for_index(&self, tool_name: &str) -> Option<Value> {
        if self.graph.read().await.node_count() > 0 {
            return None;
        }

        let task_id = self
            .tasks
            .create(tool_name, "Waiting for background index to complete")
            .await;
        let graph = self.graph.clone();
        let tasks = self.tasks.clone();
        let tid = task_id.clone();

        tokio::spawn(async move {
            tasks.set_running(&tid, "Indexing project...", 10).await;
            for i in 0..120 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let count = graph.read().await.node_count();
                let progress = ((i + 1) * 100 / 120).min(95) as u8;
                if count > 0 {
                    tasks
                        .complete(&tid, json!({ "indexed": true, "node_count": count }))
                        .await;
                    return;
                }
                tasks
                    .set_running(&tid, &format!("Indexing... ({}s)", (i + 1) / 2), progress)
                    .await;
            }
            tasks
                .fail(&tid, "Indexing timed out after 60 seconds")
                .await;
        });

        Some(tasks::TaskManager::task_handle_response(&task_id))
    }

    async fn handle_request(&self, req: JsonRpcRequest) -> Option<JsonRpcResponse> {
        let id = req.id.clone();
        let params = req.params.clone().unwrap_or(Value::Null);
        let meta = parse_request_meta(&params);
        let negotiated = self.negotiated_protocol.read().await.clone();
        let protocol = resolve_protocol_version(&meta, negotiated.as_deref());

        let result = match req.method.as_str() {
            "initialize" => {
                let client_version = params
                    .get("protocolVersion")
                    .and_then(|v| v.as_str())
                    .unwrap_or(PROTOCOL_VERSION_LEGACY);
                *self.negotiated_protocol.write().await = Some(client_version.to_string());

                let caps = if client_version == PROTOCOL_VERSION_LATEST
                    || client_version.starts_with("2026-")
                {
                    server_capabilities()
                } else {
                    legacy_capabilities()
                };

                Ok(json!({
                    "protocolVersion": if client_version.starts_with("2026-") {
                        PROTOCOL_VERSION_LATEST
                    } else {
                        PROTOCOL_VERSION_LEGACY
                    },
                    "capabilities": caps,
                    "serverInfo": {
                        "name": "arbor-mcp",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }))
            }
            "server/discover" => Ok(discover_response()),
            "notifications/initialized" => Ok(json!({})),
            "tools/list" => match self.list_tools() {
                Ok(mut result) => {
                    if protocol == PROTOCOL_VERSION_LATEST {
                        result = with_cache_meta(result, DEFAULT_TTL_MS);
                    }
                    Ok(result)
                }
                Err(e) => Err(e),
            },
            "tools/call" => self.call_tool(params).await,
            "resources/list" => match self.list_resources() {
                Ok(mut result) => {
                    if protocol == PROTOCOL_VERSION_LATEST {
                        result = with_cache_meta(result, DEFAULT_TTL_MS);
                    }
                    Ok(result)
                }
                Err(e) => Err(e),
            },
            "resources/read" => match self.read_resource(params).await {
                Ok(mut result) => {
                    if protocol == PROTOCOL_VERSION_LATEST {
                        result = with_cache_meta(result, DEFAULT_TTL_MS);
                    }
                    Ok(result)
                }
                Err(e) => Err(e),
            },
            "tasks/get" => self.tasks_get(params).await,
            "tasks/update" => self.tasks_update(params).await,
            "tasks/cancel" => self.tasks_cancel(params).await,
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

    async fn tasks_get(&self, params: Value) -> Result<Value, JsonRpcError> {
        let task_id = params
            .get("taskId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing 'taskId' parameter".to_string(),
                data: None,
            })?;

        self.tasks
            .get_response(task_id)
            .await
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: format!("Task not found: {}", task_id),
                data: None,
            })
    }

    async fn tasks_update(&self, params: Value) -> Result<Value, JsonRpcError> {
        let task_id = params
            .get("taskId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing 'taskId' parameter".to_string(),
                data: None,
            })?;

        self.tasks
            .update_response(task_id)
            .await
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: format!("Task not found: {}", task_id),
                data: None,
            })
    }

    async fn tasks_cancel(&self, params: Value) -> Result<Value, JsonRpcError> {
        let task_id = params
            .get("taskId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing 'taskId' parameter".to_string(),
                data: None,
            })?;

        let cancelled = self.tasks.cancel(task_id).await;
        Ok(json!({ "taskId": task_id, "cancelled": cancelled }))
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

    fn list_resources(&self) -> Result<Value, JsonRpcError> {
        let mut resources = vec![
            json!({
                "uri": "arbor://graph/stats",
                "name": "Graph Statistics",
                "description": "Node count, edge count, languages, and files indexed",
                "mimeType": "application/json"
            }),
            json!({
                "uri": "arbor://graph/entry-points",
                "name": "Entry Points",
                "description": "All detected entry points in the codebase",
                "mimeType": "application/json"
            }),
            json!({
                "uri": "arbor://graph/hotspots",
                "name": "Code Hotspots",
                "description": "Top 20 most central nodes in the codebase",
                "mimeType": "application/json"
            }),
        ];
        resources.extend(apps::list_app_resources());
        Ok(json!({ "resources": resources }))
    }

    async fn read_resource(&self, params: Value) -> Result<Value, JsonRpcError> {
        let uri = params
            .get("uri")
            .and_then(|v| v.as_str())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "Missing 'uri' parameter".to_string(),
                data: None,
            })?;

        let graph = self.graph.read().await;

        let contents = match uri {
            "arbor://graph/stats" => {
                let stats = graph.stats();
                json!({
                    "node_count": stats.node_count,
                    "edge_count": stats.edge_count,
                    "file_count": stats.files,
                })
            }
            "arbor://graph/entry-points" => {
                let eps = graph.list_entry_points();
                let entries: Vec<Value> = eps
                    .iter()
                    .map(|n| {
                        json!({
                            "id": n.id,
                            "name": n.name,
                            "kind": n.kind.to_string(),
                            "file": n.file
                        })
                    })
                    .collect();
                json!({ "entry_points": entries })
            }
            "arbor://graph/hotspots" => {
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
                    .take(20)
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
                json!({ "hotspots": hotspots })
            }
            uri if uri.starts_with("ui://") => {
                if let Some(html) = apps::read_app_template(uri) {
                    return Ok(json!({
                        "contents": [{
                            "uri": uri,
                            "mimeType": "text/html",
                            "text": html
                        }]
                    }));
                }
                return Err(JsonRpcError {
                    code: -32602,
                    message: format!("Unknown UI template: {}", uri),
                    data: None,
                });
            }
            _ => {
                return Err(JsonRpcError {
                    code: -32602,
                    message: format!("Unknown resource URI: {}", uri),
                    data: None,
                })
            }
        };

        Ok(json!({
            "contents": [
                {
                    "uri": uri,
                    "mimeType": "application/json",
                    "text": serde_json::to_string_pretty(&contents).unwrap_or_default()
                }
            ]
        }))
    }
}

#[cfg(test)]
mod tool_tests {
    use super::*;
    use arbor_graph::ArborGraph;
    use arbor_server::SharedGraph;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn empty_server() -> McpServer {
        let mut graph = ArborGraph::new();
        // Add a dummy node so the "still indexing" guard passes
        let node = arbor_core::CodeNode::new(
            "_dummy",
            "_dummy",
            arbor_core::NodeKind::Function,
            "_dummy.rs",
        );
        graph.add_node(node);
        let shared: SharedGraph = Arc::new(RwLock::new(graph));
        // Use repo root so git-diff blast radius works in tests
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        McpServer::with_project(shared, project_root)
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

    #[tokio::test]
    async fn test_explain_symbol_not_found() {
        let server = empty_server();
        let result = server
            .call_tool(serde_json::json!({
                "name": "explain_symbol", "arguments": { "symbol": "nonexistent" }
            }))
            .await;
        let val = result.unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["ok"], false);
    }

    #[tokio::test]
    async fn test_audit_security_not_found() {
        let server = empty_server();
        let result = server
            .call_tool(serde_json::json!({
                "name": "audit_security", "arguments": { "source": "nonexistent" }
            }))
            .await;
        let val = result.unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["ok"], false);
    }

    #[tokio::test]
    async fn test_get_architecture_overview_empty_graph() {
        let server = empty_server();
        let result = server
            .call_tool(serde_json::json!({
                "name": "get_architecture_overview", "arguments": {}
            }))
            .await;
        let val = result.unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["ok"], true);
    }

    #[tokio::test]
    async fn test_batch_query_returns_envelope() {
        let server = empty_server();
        let result = server
            .call_tool(serde_json::json!({
                "name": "batch_query", "arguments": { "symbols": ["nonexistent"] }
            }))
            .await;
        let val = result.unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["ok"], true);
    }

    #[tokio::test]
    async fn test_get_blast_radius_returns_envelope() {
        let server = empty_server();
        let result = server
            .call_tool(serde_json::json!({
                "name": "get_blast_radius", "arguments": {}
            }))
            .await;
        let val = result.unwrap();
        let text = val["content"][0]["text"].as_str().unwrap();
        let envelope: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(envelope["ok"], true);
        assert!(envelope["data"]["risk_level"].is_string());
    }

    #[tokio::test]
    async fn test_server_discover() {
        let server = empty_server();
        let resp = server
            .handle_request(JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                method: "server/discover".to_string(),
                params: None,
                id: Some(json!(1)),
            })
            .await
            .unwrap();
        assert_eq!(resp.result.unwrap()["protocolVersion"], "2026-07-28");
    }

    #[tokio::test]
    async fn test_read_ui_template_resource() {
        let server = empty_server();
        let result = server
            .read_resource(json!({ "uri": apps::UI_BLAST_RADIUS }))
            .await
            .unwrap();
        let text = result["contents"][0]["text"].as_str().unwrap();
        assert!(text.contains("canvas"));
    }
}
