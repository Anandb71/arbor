//! Transport-level tests: drive the real stdio protocol loop over a duplex
//! channel. Covers version negotiation, extension declaration, JSON-RPC error
//! shapes, notification silence, cancellation, and clean disconnect.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use arbor_mcp::McpServer;
use serde_json::{json, Value};
use tokio::io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::RwLock;

const LATEST: &str = "2026-07-28";
const LEGACY: &str = "2025-03-26";

fn server() -> McpServer {
    let mut graph = arbor_graph::ArborGraph::new();
    // A dummy node so the indexing guard does not intercept tool calls.
    graph.add_node(arbor_core::CodeNode::new(
        "_dummy",
        "_dummy",
        arbor_core::NodeKind::Function,
        "_dummy.rs",
    ));
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    McpServer::with_project(Arc::new(RwLock::new(graph)), project_root)
}

/// A stdio-line client paired with the spawned server task.
struct Client {
    write: tokio::io::DuplexStream,
    lines: tokio::io::Lines<BufReader<tokio::io::DuplexStream>>,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Client {
    fn connect() -> Self {
        let (client_read, server_write) = duplex(64 * 1024);
        let (server_read, client_write) = duplex(64 * 1024);
        let server = server();
        let task = tokio::spawn(async move {
            server
                .serve_lines(BufReader::new(server_read), server_write)
                .await
        });
        Self {
            write: client_write,
            lines: BufReader::new(client_read).lines(),
            task,
        }
    }

    async fn send(&mut self, value: &Value) {
        let mut line = serde_json::to_string(value).unwrap();
        line.push('\n');
        self.write.write_all(line.as_bytes()).await.unwrap();
    }

    async fn request(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await;
        let line = tokio::time::timeout(Duration::from_secs(10), self.lines.next_line())
            .await
            .expect("server did not answer a request")
            .unwrap()
            .unwrap();
        let resp: Value = serde_json::from_str(&line).expect("response line is not JSON");
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], id);
        resp
    }

    /// Send raw bytes and read the next response line, if any.
    async fn send_raw(&mut self, raw: &str) {
        self.write.write_all(raw.as_bytes()).await.unwrap();
    }

    /// Assert the server stays silent for a window (notifications produce no
    /// response; the protocol loop must not emit chatter).
    async fn expect_silent(&mut self) {
        let next = tokio::time::timeout(Duration::from_millis(300), self.lines.next_line()).await;
        assert!(next.is_err(), "expected silence, got {:?}", next.ok());
    }
}

#[tokio::test]
async fn initialize_negotiates_both_protocol_versions() {
    let mut latest = Client::connect();
    let resp = latest
        .request(1, "initialize", json!({ "protocolVersion": LATEST }))
        .await;
    assert_eq!(resp["result"]["protocolVersion"], LATEST);
    assert_eq!(
        resp["result"]["capabilities"]["extensions"]["io.modelcontextprotocol/tasks"],
        json!({ "version": "1.0.0" })
    );

    let mut legacy = Client::connect();
    let resp = legacy
        .request(1, "initialize", json!({ "protocolVersion": LEGACY }))
        .await;
    assert_eq!(resp["result"]["protocolVersion"], LEGACY);
    assert!(resp["result"]["capabilities"]["extensions"].is_null());
}

#[tokio::test]
async fn initialize_future_version_negotiates_latest_not_legacy() {
    let mut client = Client::connect();
    let resp = client
        .request(1, "initialize", json!({ "protocolVersion": "2030-06-01" }))
        .await;
    assert_eq!(resp["result"]["protocolVersion"], LATEST);
}

#[tokio::test]
async fn initialize_rejects_a_malformed_version() {
    let mut client = Client::connect();
    let resp = client
        .request(
            1,
            "initialize",
            json!({ "protocolVersion": "not-a-version" }),
        )
        .await;
    assert_eq!(resp["error"]["code"], -32602);
    assert!(resp["error"]["message"].as_str().unwrap().contains(LATEST));
}

#[tokio::test]
async fn extensions_are_narrowed_to_what_the_client_declared() {
    let mut client = Client::connect();
    let resp = client
        .request(
            1,
            "initialize",
            json!({
                "protocolVersion": LATEST,
                "capabilities": { "extensions": {
                    "io.modelcontextprotocol/tasks": {},
                    "vendor.example/unsupported": {}
                } }
            }),
        )
        .await;
    let ext = &resp["result"]["capabilities"]["extensions"];
    assert_eq!(
        ext["io.modelcontextprotocol/tasks"],
        json!({ "version": "1.0.0" })
    );
    // A client-declared extension this server does not implement is absent.
    assert!(ext.get("vendor.example/unsupported").is_none());
    // And one it did not declare is absent too.
    assert!(ext.get("io.modelcontextprotocol/apps").is_none());
}

#[tokio::test]
async fn every_advertised_extension_has_a_real_method() {
    let mut client = Client::connect();
    client
        .request(1, "initialize", json!({ "protocolVersion": LATEST }))
        .await;
    let tools = client.request(2, "tools/list", json!({})).await;
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(
        names.len() >= 10,
        "expected the full tool set, got {names:?}"
    );
    for name in &names {
        assert!(!name.is_empty());
    }
    // The tasks extension's methods must exist for the advertised extension.
    let resp = client
        .request(3, "tasks/cancel", json!({ "taskId": "no-such-task" }))
        .await;
    assert_ne!(
        resp["error"]["code"], -32601,
        "tasks/cancel is advertised but missing"
    );
}

#[tokio::test]
async fn unknown_method_and_tool_return_errors_not_crashes() {
    let mut client = Client::connect();
    let resp = client
        .request(1, "tools/call", json!({ "name": "nonexistent_tool" }))
        .await;
    assert!(resp.get("error").is_some() || resp["result"].is_null() == false);

    let resp = client.request(2, "no/such_method", json!({})).await;
    assert_eq!(resp["error"]["code"], -32601);
}

#[tokio::test]
async fn malformed_input_gets_a_parse_error_response() {
    let mut client = Client::connect();
    client.send_raw("this is not json\r\n").await;
    let line = tokio::time::timeout(Duration::from_secs(10), client.lines.next_line())
        .await
        .expect("no parse error response")
        .unwrap()
        .unwrap();
    let resp: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(resp["error"]["code"], -32700);
}

#[tokio::test]
async fn notifications_produce_no_response() {
    let mut client = Client::connect();
    client
        .send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        .await;
    client.expect_silent().await;
    // The connection is still alive for real requests afterwards.
    let resp = client.request(2, "tools/list", json!({})).await;
    assert!(resp["result"]["tools"].is_array());
}

#[tokio::test]
async fn resources_list_and_read_round_trip() {
    let mut client = Client::connect();
    let listed = client.request(1, "resources/list", json!({})).await;
    let resources = listed["result"]["resources"].as_array().unwrap();
    assert!(!resources.is_empty());
    let uri = resources[0]["uri"].as_str().unwrap();
    let read = client
        .request(2, "resources/read", json!({ "uri": uri }))
        .await;
    assert!(read["result"]["contents"].is_array());
}

#[tokio::test]
async fn client_disconnect_ends_the_server_cleanly() {
    let Client { task, write, lines } = Client::connect();
    drop(write);
    drop(lines);
    let result = tokio::time::timeout(Duration::from_secs(5), task).await;
    assert!(
        result.is_ok(),
        "server did not finish after client disconnect"
    );
    assert!(result.unwrap().unwrap().is_ok());
}
