//! MCP protocol constants and helpers for MCP 2026-07-28.

use serde_json::{json, Value};

/// Latest MCP protocol version supported by Arbor.
pub const PROTOCOL_VERSION_LATEST: &str = "2026-07-28";

/// Legacy protocol version for backward compatibility.
pub const PROTOCOL_VERSION_LEGACY: &str = "2025-03-26";

/// Tasks extension identifier.
pub const EXT_TASKS: &str = "io.modelcontextprotocol/tasks";

/// MCP Apps extension identifier.
pub const EXT_APPS: &str = "io.modelcontextprotocol/apps";

/// Default TTL for cacheable list/read responses (5 minutes).
pub const DEFAULT_TTL_MS: u64 = 300_000;

/// Client metadata extracted from `_meta` on stateless requests.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct RequestMeta {
    pub protocol_version: Option<String>,
    pub client_name: Option<String>,
    pub client_version: Option<String>,
}

/// Parse `_meta` from JSON-RPC params (stateless MCP 2026-07-28).
pub fn parse_request_meta(params: &Value) -> RequestMeta {
    let meta = params.get("_meta").or_else(|| params.get("meta"));
    let Some(meta) = meta else {
        return RequestMeta::default();
    };

    RequestMeta {
        protocol_version: meta
            .get("protocolVersion")
            .or_else(|| meta.get("protocol_version"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        client_name: meta
            .get("clientInfo")
            .and_then(|c| c.get("name"))
            .or_else(|| meta.get("clientName"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        client_version: meta
            .get("clientInfo")
            .and_then(|c| c.get("version"))
            .or_else(|| meta.get("clientVersion"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
    }
}

/// Resolve protocol version: prefer client `_meta`, fall back to initialize negotiation.
pub fn resolve_protocol_version(meta: &RequestMeta, negotiated: Option<&str>) -> &'static str {
    if let Some(v) = meta.protocol_version.as_deref() {
        if v == PROTOCOL_VERSION_LATEST || v.starts_with("2026-") {
            return PROTOCOL_VERSION_LATEST;
        }
    }
    if let Some(v) = negotiated {
        if v == PROTOCOL_VERSION_LATEST || v.starts_with("2026-") {
            return PROTOCOL_VERSION_LATEST;
        }
    }
    PROTOCOL_VERSION_LEGACY
}

/// Attach caching metadata to list/read responses per SEP-2549.
pub fn with_cache_meta(mut value: Value, ttl_ms: u64) -> Value {
    if let Some(obj) = value.as_object_mut() {
        obj.insert("ttlMs".to_string(), json!(ttl_ms));
        obj.insert("cacheScope".to_string(), json!("server"));
    }
    value
}

/// Server capabilities for MCP 2026-07-28.
pub fn server_capabilities() -> Value {
    json!({
        "tools": { "listChanged": false },
        "resources": { "subscribe": false, "listChanged": false },
        "extensions": {
            EXT_TASKS: { "version": "1.0.0" },
            EXT_APPS: { "version": "1.0.0" }
        },
        "streaming": false,
        "pagination": true,
        "json": true
    })
}

/// Legacy capabilities for 2025-03-26 clients.
pub fn legacy_capabilities() -> Value {
    json!({
        "tools": {},
        "resources": {},
        "streaming": false,
        "pagination": false,
        "json": true
    })
}

/// `server/discover` response for stateless clients.
pub fn discover_response() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION_LATEST,
        "capabilities": server_capabilities(),
        "serverInfo": {
            "name": "arbor-mcp",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Graph-native code intelligence — first MCP server for 2026-07-28"
        },
        "extensions": [
            { "id": EXT_TASKS, "version": "1.0.0", "description": "Long-running index and audit tasks" },
            { "id": EXT_APPS, "version": "1.0.0", "description": "Interactive blast-radius and architecture graph UIs" }
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_meta_from_params() {
        let params = json!({
            "_meta": {
                "protocolVersion": "2026-07-28",
                "clientInfo": { "name": "cursor", "version": "1.0" }
            }
        });
        let meta = parse_request_meta(&params);
        assert_eq!(meta.protocol_version.as_deref(), Some("2026-07-28"));
        assert_eq!(meta.client_name.as_deref(), Some("cursor"));
    }

    #[test]
    fn with_cache_meta_adds_fields() {
        let val = with_cache_meta(json!({ "tools": [] }), 60_000);
        assert_eq!(val["ttlMs"], 60_000);
        assert_eq!(val["cacheScope"], "server");
    }
}
