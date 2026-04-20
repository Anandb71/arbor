//! WebSocket server implementation.
//!
//! Handles client connections and routes messages to handlers.

use crate::handlers::{
    handle_context, handle_discover, handle_impact, handle_info, handle_node_get, handle_search,
    SharedGraph,
};
use crate::protocol::{
    ContextParams, DiscoverParams, ImpactParams, NodeGetParams, Request, Response, SearchParams,
};
use arbor_graph::ArborGraph;
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

/// Server configuration.
pub struct ServerConfig {
    /// Address to bind to.
    pub addr: SocketAddr,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            addr: SocketAddr::from(([127, 0, 0, 1], 7432)),
        }
    }
}

/// The Arbor WebSocket server.
pub struct ArborServer {
    config: ServerConfig,
    graph: SharedGraph,
}

impl ArborServer {
    /// Creates a new server with the given graph.
    pub fn new(graph: ArborGraph, config: ServerConfig) -> Self {
        Self {
            config,
            graph: Arc::new(RwLock::new(graph)),
        }
    }

    /// Creates a new server with an existing shared graph handle.
    pub fn new_with_shared(graph: SharedGraph, config: ServerConfig) -> Self {
        Self { config, graph }
    }

    /// Returns a handle to the shared graph for updates.
    pub fn graph(&self) -> SharedGraph {
        self.graph.clone()
    }

    /// Runs the server, accepting connections forever.
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind(&self.config.addr).await?;
        info!("Arbor server listening on {}", self.config.addr);

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    debug!("New connection from {}", addr);
                    let graph = self.graph.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, addr, graph).await {
                            error!("Connection error from {}: {}", addr, e);
                        }
                    });
                }
                Err(e) => {
                    error!("Accept error: {}", e);
                }
            }
        }
    }
}

/// Handles a single WebSocket connection.
async fn handle_connection(
    stream: TcpStream,
    addr: SocketAddr,
    graph: SharedGraph,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let ws_stream = accept_async(stream).await?;
    info!("WebSocket connection established with {}", addr);

    let (mut write, mut read) = ws_stream.split();

    while let Some(msg) = read.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                warn!("Message error from {}: {}", addr, e);
                break;
            }
        };

        if msg.is_close() {
            debug!("Client {} disconnected", addr);
            break;
        }

        if msg.is_ping() {
            write.send(Message::Pong(msg.into_data())).await?;
            continue;
        }

        if msg.is_text() {
            let text = msg.to_text().unwrap_or("");
            let response = process_message(text, graph.clone()).await;
            let json = serde_json::to_string(&response)?;
            write.send(Message::Text(json)).await?;
        }
    }

    info!("Connection closed: {}", addr);
    Ok(())
}

/// Processes a JSON-RPC message and returns a response.
async fn process_message(text: &str, graph: SharedGraph) -> Response {
    // Parse the request
    let request: Request = match serde_json::from_str(text) {
        Ok(r) => r,
        Err(_) => return Response::parse_error(),
    };

    let id = request.id.clone();
    let method = request.method.as_str();

    debug!("Processing method: {}", method);

    // Route to handler
    match method {
        "graph.info" => handle_info(graph, id).await,

        "discover" => match serde_json::from_value::<DiscoverParams>(request.params) {
            Ok(params) => handle_discover(graph, id, params).await,
            Err(e) => Response::invalid_params(id, e.to_string()),
        },

        "impact" => match serde_json::from_value::<ImpactParams>(request.params) {
            Ok(params) => handle_impact(graph, id, params).await,
            Err(e) => Response::invalid_params(id, e.to_string()),
        },

        "context" => match serde_json::from_value::<ContextParams>(request.params) {
            Ok(params) => handle_context(graph, id, params).await,
            Err(e) => Response::invalid_params(id, e.to_string()),
        },

        "search" => match serde_json::from_value::<SearchParams>(request.params) {
            Ok(params) => handle_search(graph, id, params).await,
            Err(e) => Response::invalid_params(id, e.to_string()),
        },

        "node.get" => match serde_json::from_value::<NodeGetParams>(request.params) {
            Ok(params) => handle_node_get(graph, id, params).await,
            Err(e) => Response::invalid_params(id, e.to_string()),
        },

        _ => Response::method_not_found(id, method),
    }
}
