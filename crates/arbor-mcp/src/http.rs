//! Streamable HTTP transport for MCP 2026-07-28 (stateless, local only).
//!
//! The server binds to loopback and must answer only local clients. A web page
//! on another site must not reach it, directly or through DNS rebinding, so:
//!
//! - a present `Origin` must be a loopback origin, otherwise `403` (required by
//!   the transport specification);
//! - `Host` must name a loopback host, since a rebinding page's requests carry
//!   the attacker's hostname;
//! - MCP messages must be `application/json`, which a cross-origin page cannot
//!   send without a CORS preflight, and no CORS headers are ever returned.
//!
//! HTTP framing comes from hyper. Header and body deadlines, the body size and
//! the number of concurrent connections are bounded.

use crate::{HttpReply, McpServer};
use anyhow::Result;
use bytes::Bytes;
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use hyper::body::Body;
use hyper::header::{HeaderValue, ALLOW, CACHE_CONTROL, CONTENT_TYPE, HOST, ORIGIN};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;

const HEALTH: &str = r#"{"status":"ok","server":"arbor-mcp","protocol":"2026-07-28"}"#;

/// Resource bounds for the transport.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub max_body_bytes: usize,
    pub header_timeout: Duration,
    pub body_timeout: Duration,
    pub max_connections: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_body_bytes: 4 * 1024 * 1024,
            header_timeout: Duration::from_secs(10),
            body_timeout: Duration::from_secs(30),
            max_connections: 64,
        }
    }
}

/// Run the MCP HTTP server on `127.0.0.1:port`.
pub async fn run_http_server(server: Arc<McpServer>, port: u16) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    eprintln!(
        "Arbor MCP HTTP listening on http://127.0.0.1:{}/mcp",
        listener.local_addr()?.port()
    );
    serve(listener, server, Limits::default()).await
}

pub(crate) async fn serve(
    listener: TcpListener,
    server: Arc<McpServer>,
    limits: Limits,
) -> Result<()> {
    let slots = Arc::new(Semaphore::new(limits.max_connections));
    loop {
        // Take a slot before accepting, so excess clients wait in the kernel's
        // accept queue instead of becoming unbounded tasks.
        let permit = slots.clone().acquire_owned().await?;
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                // A transient failure (such as too many open files) must not
                // stop the server.
                eprintln!("MCP HTTP accept error: {}", error);
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let server = server.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let service = service_fn(move |request| {
                let server = server.clone();
                async move { Ok::<_, Infallible>(handle(request, &server, limits).await) }
            });
            let connection = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(limits.header_timeout)
                .keep_alive(false)
                .serve_connection(TokioIo::new(stream), service);
            // Timeouts and aborted requests from misbehaving clients end here.
            let _ = connection.await;
        });
    }
}

async fn handle<B>(request: Request<B>, server: &McpServer, limits: Limits) -> Response<Full<Bytes>>
where
    B: Body<Data = Bytes>,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    if !host_is_loopback(request.headers().get(HOST)) {
        return rpc_error(
            StatusCode::FORBIDDEN,
            -32600,
            "Host is not a loopback address",
        );
    }
    if let Some(origin) = request.headers().get(ORIGIN) {
        if !origin_is_loopback(origin) {
            return rpc_error(StatusCode::FORBIDDEN, -32600, "Origin not allowed");
        }
    }

    match (request.method(), request.uri().path()) {
        (&Method::GET, "/health") | (&Method::GET, "/") => {
            return json(StatusCode::OK, HEALTH.to_string())
        }
        (&Method::POST, "/mcp") | (&Method::POST, "/") => {}
        (_, "/mcp") => {
            let mut response = rpc_error(
                StatusCode::METHOD_NOT_ALLOWED,
                -32600,
                "The MCP endpoint accepts POST only",
            );
            response
                .headers_mut()
                .insert(ALLOW, HeaderValue::from_static("POST"));
            return response;
        }
        _ => {
            return json(
                StatusCode::NOT_FOUND,
                r#"{"error":"not found"}"#.to_string(),
            )
        }
    }

    if !is_json(request.headers().get(CONTENT_TYPE)) {
        return rpc_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            -32600,
            "MCP messages must be sent as Content-Type: application/json",
        );
    }

    let body = Limited::new(request.into_body(), limits.max_body_bytes);
    let bytes = match tokio::time::timeout(limits.body_timeout, body.collect()).await {
        Err(_) => {
            return rpc_error(
                StatusCode::REQUEST_TIMEOUT,
                -32600,
                "Request body not received in time",
            )
        }
        Ok(Err(error)) if error.downcast_ref::<LengthLimitError>().is_some() => {
            return rpc_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                -32600,
                "Request body is too large",
            )
        }
        Ok(Err(_)) => {
            return rpc_error(
                StatusCode::BAD_REQUEST,
                -32600,
                "Request body could not be read",
            )
        }
        Ok(Ok(collected)) => collected.to_bytes(),
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return rpc_error(
            StatusCode::BAD_REQUEST,
            -32700,
            "Parse error: body is not UTF-8",
        );
    };

    match server.handle_http_message(text).await {
        HttpReply::Accepted => {
            let mut response = Response::new(Full::new(Bytes::new()));
            *response.status_mut() = StatusCode::ACCEPTED;
            response
        }
        HttpReply::Json(status, body) => json(
            StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            body,
        ),
    }
}

/// `localhost`, `127.0.0.1` or `[::1]`, with or without a port.
fn is_loopback_authority(authority: &str) -> bool {
    let host = if let Some(rest) = authority.strip_prefix('[') {
        match rest.split_once(']') {
            Some((host, port)) if port.is_empty() || valid_port(port) => host,
            _ => return false,
        }
    } else {
        match authority.split_once(':') {
            Some((host, port)) if valid_port(&format!(":{}", port)) => host,
            Some(_) => return false,
            None => authority,
        }
    };
    matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

fn valid_port(port: &str) -> bool {
    port.strip_prefix(':')
        .is_some_and(|digits| !digits.is_empty() && digits.parse::<u16>().is_ok())
}

fn host_is_loopback(host: Option<&HeaderValue>) -> bool {
    host.and_then(|value| value.to_str().ok())
        .is_some_and(is_loopback_authority)
}

fn origin_is_loopback(origin: &HeaderValue) -> bool {
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let authority = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"));
    authority.is_some_and(|authority| !authority.contains('/') && is_loopback_authority(authority))
}

fn is_json(content_type: Option<&HeaderValue>) -> bool {
    content_type
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
}

fn json(status: StatusCode, body: String) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// A JSON-RPC error with no `id`, for failures before a message is read.
fn rpc_error(status: StatusCode, code: i32, message: &str) -> Response<Full<Bytes>> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "error": { "code": code, "message": message },
        "id": null
    });
    json(status, body.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arbor_graph::ArborGraph;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::sync::RwLock;

    const LIST: &str = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;

    fn server() -> McpServer {
        McpServer::new(Arc::new(RwLock::new(ArborGraph::new())))
    }

    fn request(
        method: Method,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> Request<Full<Bytes>> {
        let mut builder = Request::builder().method(method).uri(path);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder
            .body(Full::new(Bytes::from(body.to_string())))
            .unwrap()
    }

    fn post(headers: &[(&str, &str)], body: &str) -> Request<Full<Bytes>> {
        request(Method::POST, "/mcp", headers, body)
    }

    async fn send(req: Request<Full<Bytes>>) -> (StatusCode, hyper::HeaderMap, String) {
        let response = handle(req, &server(), Limits::default()).await;
        let (parts, body) = response.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        (
            parts.status,
            parts.headers,
            String::from_utf8(bytes.to_vec()).unwrap(),
        )
    }

    const LOCAL: (&str, &str) = ("host", "127.0.0.1:7433");
    const JSON: (&str, &str) = ("content-type", "application/json");

    #[tokio::test]
    async fn local_json_request_is_answered_without_cors() {
        let (status, headers, body) = send(post(&[LOCAL, JSON], LIST)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("\"tools\""), "{body}");
        assert!(headers.get("access-control-allow-origin").is_none());
        for host in ["localhost:7433", "[::1]:7433", "LOCALHOST", "127.0.0.1"] {
            let (status, _, _) = send(post(&[("host", host), JSON], LIST)).await;
            assert_eq!(status, StatusCode::OK, "{host}");
        }
    }

    #[tokio::test]
    async fn foreign_origins_are_forbidden() {
        for origin in [
            "https://evil.example",
            "null",
            "http://127.0.0.1.evil.example",
            "http://localhost.evil:80",
            "file://",
            "http://localhost/path",
        ] {
            let (status, headers, body) =
                send(post(&[LOCAL, JSON, ("origin", origin)], LIST)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{origin}");
            assert!(body.contains("Origin not allowed"));
            assert!(headers.get("access-control-allow-origin").is_none());
        }
        let (status, _, _) = send(post(
            &[LOCAL, JSON, ("origin", "http://localhost:5173")],
            LIST,
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn dns_rebinding_hosts_are_forbidden() {
        for host in [
            "evil.example:7433",
            "127.0.0.1.nip.io",
            "localhost:notaport",
            "[::1]x",
            "",
        ] {
            let (status, _, _) = send(post(&[("host", host), JSON], LIST)).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{host:?}");
        }
        let (status, _, _) = send(post(&[JSON], LIST)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "missing Host");
    }

    #[tokio::test]
    async fn cors_simple_requests_are_refused() {
        // A cross-origin page can send text/plain or form bodies without a preflight.
        for content_type in [
            "text/plain",
            "application/x-www-form-urlencoded",
            "multipart/form-data; boundary=x",
        ] {
            let (status, _, _) = send(post(&[LOCAL, ("content-type", content_type)], LIST)).await;
            assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{content_type}");
        }
        let (status, _, _) = send(post(&[LOCAL], LIST)).await;
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        let (status, _, _) = send(post(
            &[LOCAL, ("content-type", "application/json; charset=utf-8")],
            LIST,
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn messages_map_to_transport_statuses() {
        let notification = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        let (status, _, body) = send(post(&[LOCAL, JSON], notification)).await;
        assert_eq!((status, body.as_str()), (StatusCode::ACCEPTED, ""));
        let (status, _, body) = send(post(&[LOCAL, JSON], "{not json")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("-32700"));
        let (status, _, body) = send(post(&[LOCAL, JSON], &format!("[{LIST}]"))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("-32600"));
        let (status, headers, _) = send(request(Method::GET, "/mcp", &[LOCAL], "")).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(headers.get("allow").unwrap(), "POST");
        let (status, _, _) = send(request(Method::GET, "/health", &[LOCAL], "")).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, _) = send(request(Method::GET, "/other", &[LOCAL], "")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    async fn spawn(limits: Limits) -> std::net::SocketAddr {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(serve(listener, Arc::new(server()), limits));
        address
    }

    async fn read_all(stream: &mut TcpStream) -> String {
        let mut response = Vec::new();
        let _ =
            tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response)).await;
        String::from_utf8_lossy(&response).into_owned()
    }

    #[tokio::test]
    async fn requests_split_across_packets_are_framed() {
        let address = spawn(Limits::default()).await;
        let mut stream = TcpStream::connect(address).await.unwrap();
        let head = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            address.port(),
            LIST.len()
        );
        let (first, second) = head.split_at(20);
        for piece in [first, second, &LIST[..10], &LIST[10..]] {
            stream.write_all(piece.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        let response = read_all(&mut stream).await;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains("\"tools\""), "{response}");
    }

    #[tokio::test]
    async fn chunked_bodies_are_accepted() {
        let address = spawn(Limits::default()).await;
        let mut stream = TcpStream::connect(address).await.unwrap();
        let request = format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            LIST.len(),
            LIST
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let response = read_all(&mut stream).await;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    }

    #[tokio::test]
    async fn oversized_and_stalled_requests_are_bounded() {
        let limits = Limits {
            max_body_bytes: 64,
            header_timeout: Duration::from_millis(300),
            body_timeout: Duration::from_millis(300),
            max_connections: 4,
        };
        let address = spawn(limits).await;

        let mut stream = TcpStream::connect(address).await.unwrap();
        let big = "x".repeat(1_000);
        let request = format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            big.len(),
            big
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        assert!(read_all(&mut stream).await.starts_with("HTTP/1.1 413"));

        // Headers that never finish: the connection is closed, not held open.
        let mut stalled = TcpStream::connect(address).await.unwrap();
        stalled
            .write_all(b"POST /mcp HTTP/1.1\r\nHost: localhost\r\n")
            .await
            .unwrap();
        let started = std::time::Instant::now();
        let response = read_all(&mut stalled).await;
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "stalled headers held the connection"
        );
        assert!(!response.contains("200 OK"));

        // A body that never arrives: 408 after the body deadline.
        let mut slow = TcpStream::connect(address).await.unwrap();
        slow.write_all(b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: 50\r\n\r\n{").await.unwrap();
        assert!(read_all(&mut slow).await.starts_with("HTTP/1.1 408"));
    }

    /// Write `raw` on one connection and return everything the server sends
    /// back before closing it.
    async fn exchange(address: std::net::SocketAddr, raw: &str) -> String {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(raw.as_bytes()).await.unwrap();
        read_all(&mut stream).await
    }

    fn responses(output: &str) -> usize {
        output.matches("HTTP/1.1 ").count()
    }

    const HEAD: &str =
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n";

    fn framed(body: &str) -> String {
        format!("{HEAD}Content-Length: {}\r\n\r\n{body}", body.len())
    }

    /// Keep-alive is off, so each connection carries exactly one request.
    /// That is what stops a mis-framed body from being read as a second,
    /// smuggled request, so pin it: a pipelined request gets no answer.
    #[tokio::test]
    async fn one_connection_answers_one_request() {
        let address = spawn(Limits::default()).await;
        let output = exchange(address, &format!("{}{}", framed(LIST), framed(LIST))).await;
        assert!(output.starts_with("HTTP/1.1 200"), "{output}");
        assert_eq!(responses(&output), 1, "{output}");
    }
}
