//! Streamable HTTP transport for MCP 2026-07-28 (stateless).

use crate::McpServer;
use anyhow::Result;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Run the MCP HTTP server on the given port.
pub async fn run_http_server(server: Arc<McpServer>, port: u16) -> Result<()> {
    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr).await?;
    eprintln!("Arbor MCP HTTP listening on http://{}", addr);

    loop {
        let (mut stream, _) = listener.accept().await?;
        let server = server.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(&mut stream, &server).await {
                eprintln!("MCP HTTP connection error: {}", e);
            }
        });
    }
}

async fn handle_connection(stream: &mut tokio::net::TcpStream, server: &McpServer) -> Result<()> {
    let mut buf = vec![0u8; 65536];
    let n = stream.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }

    let request = String::from_utf8_lossy(&buf[..n]);
    let (method, path, headers, body) = parse_http_request(&request);

    if method == "GET" && (path == "/health" || path == "/") {
        let body = r#"{"status":"ok","server":"arbor-mcp","protocol":"2026-07-28"}"#;
        write_http_response(stream, 200, body).await?;
        return Ok(());
    }

    if method == "POST" && (path == "/mcp" || path == "/") {
        let mcp_method = headers
            .get("mcp-method")
            .or_else(|| headers.get("Mcp-Method"))
            .cloned()
            .unwrap_or_default();
        let mcp_name = headers
            .get("mcp-name")
            .or_else(|| headers.get("Mcp-Name"))
            .cloned()
            .unwrap_or_default();

        if !mcp_method.is_empty() {
            eprintln!("MCP HTTP: method={} name={}", mcp_method, mcp_name);
        }

        let response = server.handle_http_body(&body).await;
        write_http_response(stream, 200, &response).await?;
        return Ok(());
    }

    write_http_response(stream, 404, r#"{"error":"not found"}"#).await?;
    Ok(())
}

fn parse_http_request(
    request: &str,
) -> (
    String,
    String,
    std::collections::HashMap<String, String>,
    String,
) {
    let mut lines = request.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    let method = parts.first().unwrap_or(&"GET").to_string();
    let path = parts.get(1).unwrap_or(&"/").to_string();

    let mut headers = std::collections::HashMap::new();
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }

    let body = lines.collect::<Vec<_>>().join("\r\n");
    (method, path, headers, body)
}

async fn write_http_response(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    body: &str,
) -> Result<()> {
    let status_text = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
        status,
        status_text,
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_post() {
        let req = "POST /mcp HTTP/1.1\r\nMcp-Method: tools/call\r\nMcp-Name: analyze_impact\r\nContent-Length: 10\r\n\r\n{\"json\":1}";
        let (method, path, headers, body) = parse_http_request(req);
        assert_eq!(method, "POST");
        assert_eq!(path, "/mcp");
        assert_eq!(headers.get("mcp-method").unwrap(), "tools/call");
        assert!(body.contains("json"));
    }
}
