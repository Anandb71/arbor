//! Which WebSocket handshakes the local servers accept.
//!
//! A browser lets any page open a WebSocket to `127.0.0.1` (CORS does not
//! apply to WebSockets) and sends that page's `Origin`, and a DNS-rebinding
//! page can reach the port under a hostname it controls. So, as on the MCP
//! HTTP transport:
//!
//! - a present `Origin` must be a loopback origin (`localhost`, `127.0.0.1`
//!   or `[::1]`, any port), otherwise `403`;
//! - on a server bound to a loopback address, `Host` must be a loopback
//!   authority. `arbor serve --headless` binds every interface for remote
//!   clients, so there `Host` names the machine and is not checked.
//!
//! Desktop clients, such as the VS Code extension and the visualizer, send no
//! `Origin`.

use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

/// Why a handshake is refused, or `None` when it is allowed.
pub(crate) fn refusal(request: &Request, bound_to_loopback: bool) -> Option<&'static str> {
    let headers = request.headers();
    if bound_to_loopback {
        let host = headers.get("host").and_then(|value| value.to_str().ok());
        if !host.is_some_and(is_loopback_authority) {
            return Some("Host is not a loopback address");
        }
    }
    if let Some(origin) = headers.get("origin") {
        if !origin.to_str().ok().is_some_and(is_loopback_origin) {
            return Some("Origin not allowed");
        }
    }
    None
}

/// The handshake callback: refused requests get `403` and never upgrade.
// tungstenite's handshake callback signature fixes the large error type.
#[allow(clippy::result_large_err)]
pub(crate) fn check(
    bound_to_loopback: bool,
) -> impl FnOnce(&Request, Response) -> Result<Response, ErrorResponse> {
    move |request, response| match refusal(request, bound_to_loopback) {
        None => Ok(response),
        Some(reason) => {
            let mut refused = ErrorResponse::new(Some(reason.to_string()));
            *refused.status_mut() = StatusCode::FORBIDDEN;
            Err(refused)
        }
    }
}

fn is_loopback_origin(origin: &str) -> bool {
    let authority = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"));
    authority.is_some_and(|authority| !authority.contains('/') && is_loopback_authority(authority))
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
            Some((host, port)) if valid_port(&format!(":{port}")) => host,
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

#[cfg(test)]
mod tests {
    use super::refusal;
    use tokio_tungstenite::tungstenite::handshake::server::Request;

    fn request(host: Option<&str>, origin: Option<&str>) -> Request {
        let mut builder = Request::builder().uri("/");
        if let Some(host) = host {
            builder = builder.header("host", host);
        }
        if let Some(origin) = origin {
            builder = builder.header("origin", origin);
        }
        builder.body(()).unwrap()
    }

    #[test]
    fn desktop_clients_and_local_pages_are_allowed() {
        for host in [
            "127.0.0.1:7433",
            "localhost:8081",
            "[::1]:7433",
            "LOCALHOST",
        ] {
            assert_eq!(refusal(&request(Some(host), None), true), None, "{host}");
        }
        for origin in [
            "http://localhost:3000",
            "http://127.0.0.1",
            "https://[::1]:8080",
        ] {
            let allowed = refusal(&request(Some("127.0.0.1:7433"), Some(origin)), true);
            assert_eq!(allowed, None, "{origin}");
        }
    }

    #[test]
    fn foreign_and_malformed_origins_are_refused() {
        for origin in [
            "https://evil.example",
            "null",
            "http://localhost.evil.example",
            "http://127.0.0.1.evil.example:7433",
            "http://localhost:3000/path",
            "file://",
        ] {
            let refused = refusal(&request(Some("127.0.0.1:7433"), Some(origin)), true);
            assert_eq!(refused, Some("Origin not allowed"), "{origin}");
        }
    }

    #[test]
    fn rebinding_hosts_are_refused_on_a_loopback_server() {
        for host in [Some("evil.example:7433"), Some("192.168.1.5:7433"), None] {
            let refused = refusal(&request(host, None), true);
            assert_eq!(refused, Some("Host is not a loopback address"), "{host:?}");
        }
    }

    #[test]
    fn headless_servers_accept_any_host_but_still_check_origin() {
        assert_eq!(
            refusal(&request(Some("192.168.1.5:7432"), None), false),
            None
        );
        assert_eq!(
            refusal(
                &request(Some("192.168.1.5:7432"), Some("https://evil.example")),
                false
            ),
            Some("Origin not allowed")
        );
    }
}
