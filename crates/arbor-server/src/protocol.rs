//! JSON-RPC protocol types.
//!
//! Implements the message format for the Arbor Protocol.
//! Based on JSON-RPC 2.0 with some custom extensions.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A JSON-RPC request.
#[derive(Debug, Deserialize)]
pub struct Request {
    /// JSON-RPC version (always "2.0").
    pub jsonrpc: String,

    /// Request ID for matching responses.
    pub id: Option<Value>,

    /// Method name to invoke.
    pub method: String,

    /// Method parameters.
    #[serde(default)]
    pub params: Value,
}

/// A JSON-RPC response.
#[derive(Debug, Serialize)]
pub struct Response {
    /// JSON-RPC version.
    pub jsonrpc: &'static str,

    /// Request ID this is responding to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,

    /// Result on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,

    /// Error on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    /// Creates a success response.
    pub fn success(id: Option<Value>, result: impl Serialize) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(serde_json::to_value(result).unwrap_or(Value::Null)),
            error: None,
        }
    }

    /// Creates an error response.
    pub fn error(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }

    /// Predefined error: Parse error.
    pub fn parse_error() -> Self {
        Self::error(None, -32700, "Parse error")
    }

    /// Predefined error: Invalid request.
    pub fn invalid_request(id: Option<Value>) -> Self {
        Self::error(id, -32600, "Invalid request")
    }

    /// Predefined error: Method not found.
    pub fn method_not_found(id: Option<Value>, method: &str) -> Self {
        Self::error(id, -32601, format!("Method not found: {}", method))
    }

    /// Predefined error: Invalid params.
    pub fn invalid_params(id: Option<Value>, message: impl Into<String>) -> Self {
        Self::error(id, -32602, message)
    }

    /// Predefined error: Internal error.
    pub fn internal_error(id: Option<Value>, message: impl Into<String>) -> Self {
        Self::error(id, -32603, message)
    }
}

/// A JSON-RPC error.
#[derive(Debug, Serialize)]
pub struct RpcError {
    /// Error code.
    pub code: i32,

    /// Error message.
    pub message: String,

    /// Optional additional data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Params for the discover method.
#[derive(Debug, Deserialize)]
pub struct DiscoverParams {
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

/// Params for the impact method.
#[derive(Debug, Deserialize)]
pub struct ImpactParams {
    pub node: String,
    #[serde(default = "default_depth")]
    pub depth: usize,
}

/// Params for the context method.
#[derive(Debug, Deserialize)]
pub struct ContextParams {
    pub task: String,
    #[serde(default = "default_max_tokens", rename = "maxTokens")]
    pub max_tokens: usize,
    #[serde(default, rename = "includeSource")]
    pub _include_source: bool,
}

/// Params for the search method.
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub query: String,
    pub kind: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

/// Params for node.get method.
#[derive(Debug, Deserialize)]
pub struct NodeGetParams {
    pub id: String,
}

fn default_limit() -> usize {
    10
}

fn default_depth() -> usize {
    3
}

fn default_max_tokens() -> usize {
    8000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_response_success() {
        let resp = Response::success(Some(serde_json::json!(1)), serde_json::json!({"ok": true}));
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
        assert_eq!(resp.jsonrpc, "2.0");
        assert_eq!(resp.id, Some(serde_json::json!(1)));
    }

    #[test]
    fn test_response_error() {
        let resp = Response::error(Some(serde_json::json!(2)), -32600, "Bad request");
        assert!(resp.result.is_none());
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32600);
        assert_eq!(err.message, "Bad request");
    }

    #[test]
    fn test_response_parse_error() {
        let resp = Response::parse_error();
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32700);
        assert!(resp.id.is_none());
    }

    #[test]
    fn test_response_method_not_found() {
        let resp = Response::method_not_found(Some(serde_json::json!(5)), "foo.bar");
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32601);
        assert!(err.message.contains("foo.bar"));
    }

    #[test]
    fn test_response_invalid_params() {
        let resp = Response::invalid_params(Some(serde_json::json!(3)), "missing field");
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("missing field"));
    }

    #[test]
    fn test_response_internal_error() {
        let resp = Response::internal_error(None, "something broke");
        let err = resp.error.unwrap();
        assert_eq!(err.code, -32603);
    }

    #[test]
    fn test_default_limit() {
        assert_eq!(default_limit(), 10);
    }

    #[test]
    fn test_default_depth() {
        assert_eq!(default_depth(), 3);
    }

    #[test]
    fn test_default_max_tokens() {
        assert_eq!(default_max_tokens(), 8000);
    }

    #[test]
    fn test_discover_params_deserialization() {
        let json = serde_json::json!({"query": "foo"});
        let params: DiscoverParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.query, "foo");
        assert_eq!(params.limit, 10); // default
    }

    #[test]
    fn test_impact_params_deserialization() {
        let json = serde_json::json!({"node": "main", "depth": 5});
        let params: ImpactParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.node, "main");
        assert_eq!(params.depth, 5);
    }

    #[test]
    fn test_search_params_with_kind_filter() {
        let json = serde_json::json!({"query": "user", "kind": "function", "limit": 20});
        let params: SearchParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.query, "user");
        assert_eq!(params.kind.as_deref(), Some("function"));
        assert_eq!(params.limit, 20);
    }

    #[test]
    fn test_response_serialization_roundtrip() {
        let resp = Response::success(Some(serde_json::json!(1)), "hello");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"hello\""));
        // Error field should be skipped
        assert!(!json.contains("\"error\""));
    }
}
