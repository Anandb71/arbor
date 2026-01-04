//! Arbor Server - WebSocket server for the Arbor Protocol
//!
//! This crate implements the server side of the Arbor Protocol,
//! allowing AI agents and IDE integrations to query the code graph.
//!
//! The server supports:
//! - Multiple concurrent connections
//! - JSON-RPC 2.0 messages
//! - Real-time graph updates via subscriptions
//! - File watching with debounced re-indexing

use arbor_graph::ArborGraph;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared graph state across connections.
pub type SharedGraph = Arc<RwLock<ArborGraph>>;

/// Server-to-client messages.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    GraphUpdate,
    FocusNode,
    IndexerStatus,
}

mod handlers;
mod protocol;
mod server;
pub mod sync_server;

pub use protocol::{Request, Response, RpcError};
pub use server::{ArborServer, ServerConfig};
pub use sync_server::{
    BroadcastMessage, FocusNodePayload, GraphUpdatePayload, IndexerStatusPayload, SyncServer,
    SyncServerConfig,
};
