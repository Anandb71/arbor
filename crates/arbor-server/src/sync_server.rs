//! Real-time sync server for the Arbor Visualizer.
//!
//! This module implements a WebSocket server that acts as the "Source of Truth"
//! for the visualizer. It broadcasts graph updates whenever the filesystem changes,
//! keeping the visualization in sync with the codebase.
//!
//! "Give Arbor a voice so the visualizer can hear the code breathe."

use crate::SharedGraph;
use arbor_core::ArborParser;
use arbor_graph::{ArborGraph, Edge, EdgeKind};
use futures_util::{SinkExt, StreamExt};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio_tungstenite::{accept_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration for the real-time server.
#[derive(Debug, Clone)]
pub struct SyncServerConfig {
    /// Address to bind the WebSocket server.
    pub addr: SocketAddr,
    /// Root path to watch for file changes.
    pub watch_path: PathBuf,
    /// Debounce duration for file events.
    pub debounce_ms: u64,
    /// File extensions to watch.
    pub extensions: Vec<String>,
}

impl Default for SyncServerConfig {
    fn default() -> Self {
        Self {
            addr: "127.0.0.1:8080".parse().unwrap(),
            watch_path: PathBuf::from("."),
            debounce_ms: 150,
            extensions: vec![
                "ts".into(),
                "tsx".into(),
                "js".into(),
                "jsx".into(),
                "rs".into(),
                "py".into(),
            ],
        }
    }
}

/// Server messages broadcast to all connected clients.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum BroadcastMessage {
    /// Initial handshake with server info
    Hello(HelloPayload),
    /// Start of a graph stream
    GraphBegin(GraphBeginPayload),
    /// Batch of nodes
    NodeBatch(NodeBatchPayload),
    /// Batch of edges
    EdgeBatch(EdgeBatchPayload),
    /// End of graph stream
    GraphEnd,
    /// Full graph snapshot or delta update (Legacy/Incremental)
    GraphUpdate(GraphUpdatePayload),
    /// Tell the visualizer to focus on a specific node.
    FocusNode(FocusNodePayload),
    /// Indexer progress status.
    IndexerStatus(IndexerStatusPayload),
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HelloPayload {
    pub version: String,
    pub node_count: usize,
    pub edge_count: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphBeginPayload {
    pub total_nodes: usize,
    pub total_edges: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NodeBatchPayload {
    pub nodes: Vec<arbor_core::CodeNode>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EdgeBatchPayload {
    pub edges: Vec<arbor_graph::GraphEdge>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GraphUpdatePayload {
    /// Whether this is a full snapshot or delta.
    pub is_delta: bool,
    /// Number of nodes in the graph.
    pub node_count: usize,
    /// Number of edges in the graph.
    pub edge_count: usize,
    /// Number of files indexed.
    pub file_count: usize,
    /// Changed files (for delta updates).
    pub changed_files: Vec<String>,
    /// Timestamp of the update.
    pub timestamp: u64,
    pub nodes: Option<Vec<arbor_core::CodeNode>>,
    pub edges: Option<Vec<arbor_graph::GraphEdge>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct FocusNodePayload {
    /// The node ID to focus.
    pub node_id: String,
    /// The file path containing the node.
    pub file: String,
    /// Line number in the file.
    pub line: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct IndexerStatusPayload {
    /// Current indexing phase.
    pub phase: String,
    /// Files processed so far.
    pub files_processed: usize,
    /// Total files to process.
    pub files_total: usize,
    /// Current file being processed.
    pub current_file: Option<String>,
}

/// Internal event for the file watcher.
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum WatcherEvent {
    Changed(PathBuf),
    Created(PathBuf),
    Deleted(PathBuf),
}

// ─────────────────────────────────────────────────────────────────────────────
// SyncServer
// ─────────────────────────────────────────────────────────────────────────────

/// High-performance real-time sync server.
///
/// This server:
/// - Hosts a WebSocket server for client connections
/// - Watches the filesystem for changes
/// - Debounces file events to prevent thrashing
/// - Re-parses changed files and updates the graph
/// - Broadcasts updates to all connected clients
pub struct SyncServer {
    config: SyncServerConfig,
    graph: SharedGraph,
    broadcast_tx: broadcast::Sender<BroadcastMessage>,
}

/// A cloneable handle to trigger spotlight events from external components (like MCP).
#[derive(Clone)]
pub struct SyncServerHandle {
    broadcast_tx: broadcast::Sender<BroadcastMessage>,
    graph: SharedGraph,
}

impl SyncServerHandle {
    /// Triggers a spotlight on a specific node.
    pub fn spotlight_node(&self, node_id: &str, file: &str, line: u32) {
        let msg = BroadcastMessage::FocusNode(FocusNodePayload {
            node_id: node_id.to_string(),
            file: file.to_string(),
            line,
        });
        let _ = self.broadcast_tx.send(msg);
    }

    /// Returns the shared graph for context lookups.
    pub fn graph(&self) -> SharedGraph {
        self.graph.clone()
    }
}

impl SyncServer {
    /// Creates a new sync server.
    pub fn new(config: SyncServerConfig) -> Self {
        let (broadcast_tx, _) = broadcast::channel(256);

        Self {
            config,
            graph: Arc::new(RwLock::new(ArborGraph::new())),
            broadcast_tx,
        }
    }

    /// Creates a sync server with an existing graph.
    pub fn with_graph(config: SyncServerConfig, graph: ArborGraph) -> Self {
        let (broadcast_tx, _) = broadcast::channel(256);

        Self {
            config,
            graph: Arc::new(RwLock::new(graph)),
            broadcast_tx,
        }
    }

    /// Creates a sync server with a shared graph.
    pub fn new_with_shared(config: SyncServerConfig, graph: SharedGraph) -> Self {
        let (broadcast_tx, _) = broadcast::channel(256);

        Self {
            config,
            graph,
            broadcast_tx,
        }
    }

    /// Returns a handle to the shared graph.
    pub fn graph(&self) -> SharedGraph {
        self.graph.clone()
    }

    /// Returns a broadcast receiver for server messages.
    pub fn subscribe(&self) -> broadcast::Receiver<BroadcastMessage> {
        self.broadcast_tx.subscribe()
    }

    /// Returns a cloneable handle for triggering spotlight events.
    pub fn handle(&self) -> SyncServerHandle {
        SyncServerHandle {
            broadcast_tx: self.broadcast_tx.clone(),
            graph: self.graph.clone(),
        }
    }

    /// Runs the server with file watching enabled.
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("╔═══════════════════════════════════════════════════════════╗");
        info!("║          ARBOR SYNC SERVER - THE PULSE OF CODE            ║");
        info!("╚═══════════════════════════════════════════════════════════╝");

        // Channel for watcher events
        let (watcher_tx, watcher_rx) = mpsc::channel::<WatcherEvent>(256);

        // Start the file watcher
        let watch_path = self.config.watch_path.clone();
        let extensions = self.config.extensions.clone();
        let debounce_ms = self.config.debounce_ms;

        tokio::spawn(async move {
            if let Err(e) = run_file_watcher(watch_path, extensions, debounce_ms, watcher_tx).await
            {
                error!("File watcher error: {}", e);
            }
        });

        // Start the indexer background task
        let graph = self.graph.clone();
        let broadcast_tx = self.broadcast_tx.clone();
        let watch_path = self.config.watch_path.clone();

        tokio::spawn(async move {
            run_background_indexer(watcher_rx, graph, broadcast_tx, watch_path).await;
        });

        // Start accepting WebSocket connections
        self.run_websocket_server().await
    }

    /// Runs just the WebSocket server (no file watching).
    async fn run_websocket_server(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind(&self.config.addr).await?;
        info!("🌐 WebSocket server listening on ws://{}", self.config.addr);
        info!("👁️  Watching: {}", self.config.watch_path.display());
        info!("⏱️  Debounce: {}ms", self.config.debounce_ms);

        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    info!("🔌 New connection from {}", addr);
                    let graph = self.graph.clone();
                    let broadcast_rx = self.broadcast_tx.subscribe();

                    tokio::spawn(async move {
                        if let Err(e) = handle_client(stream, addr, graph, broadcast_rx).await {
                            warn!("Connection error from {}: {}", addr, e);
                        }
                    });
                }
                Err(e) => {
                    error!("Accept error: {}", e);
                }
            }
        }
    }

    /// Broadcasts a focus command to all clients.
    pub fn focus_node(&self, node_id: &str, file: &str, line: u32) {
        let msg = BroadcastMessage::FocusNode(FocusNodePayload {
            node_id: node_id.to_string(),
            file: file.to_string(),
            line,
        });

        let _ = self.broadcast_tx.send(msg);
    }

    /// Broadcasts an indexer status update.
    pub fn update_status(
        &self,
        phase: &str,
        processed: usize,
        total: usize,
        current: Option<&str>,
    ) {
        let msg = BroadcastMessage::IndexerStatus(IndexerStatusPayload {
            phase: phase.to_string(),
            files_processed: processed,
            files_total: total,
            current_file: current.map(|s| s.to_string()),
        });

        let _ = self.broadcast_tx.send(msg);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Client Connection Handler
// ─────────────────────────────────────────────────────────────────────────────

/// Handles a single WebSocket client connection.
async fn handle_client(
    stream: TcpStream,
    addr: SocketAddr,
    graph: SharedGraph,
    mut broadcast_rx: broadcast::Receiver<BroadcastMessage>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

    let config = WebSocketConfig {
        max_send_queue: None,
        max_message_size: Some(64 * 1024 * 1024), // 64 MB
        max_frame_size: Some(64 * 1024 * 1024),   // 64 MB
        accept_unmasked_frames: false,
        ..Default::default()
    };

    let ws_stream = tokio_tungstenite::accept_async_with_config(stream, Some(config)).await?;
    let (mut write, mut read) = ws_stream.split();

    info!("✅ WebSocket handshake complete with {}", addr);

    // 1. Send Hello (Metadata)
    let (node_count, edge_count, nodes, edges) = {
        let g = graph.read().await;
        let mut nodes: Vec<_> = g.nodes().cloned().collect();
        let edges_raw = g.export_edges();
        // Sort for deterministic output (run twice = identical)
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        let mut edges = edges_raw;
        edges.sort_by(|a, b| (&a.source, &a.target).cmp(&(&b.source, &b.target)));
        (g.node_count(), g.edge_count(), nodes, edges)
    };

    let hello = BroadcastMessage::Hello(HelloPayload {
        version: "1.1.1".to_string(),
        node_count,
        edge_count,
    });

    let json = serde_json::to_string(&hello)?;
    write.send(Message::Text(json)).await?;
    info!(
        "👋 Sent Hello ({} nodes, {} edges) to {}",
        node_count, edge_count, addr
    );

    // 2. Wait for Client Ready
    info!("⏳ Waiting for client {} to be ready...", addr);
    let mut ready = false;
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                // Simple parsing for "ready_for_graph"
                if text.contains("ready_for_graph") {
                    ready = true;
                    info!("✅ Client {} is ready for graph", addr);
                    break;
                }
                debug!("Running pre-ready protocol with {}: {}", addr, text);
            }
            Ok(Message::Ping(data)) => {
                write.send(Message::Pong(data)).await?;
            }
            Ok(Message::Close(_)) => return Ok(()),
            Err(e) => return Err(e.into()),
            _ => {}
        }
    }

    if !ready {
        warn!("Client {} disconnected before sending ready signal", addr);
        return Ok(());
    }

    // 3. Stream Graph (Chunked)
    let begin = BroadcastMessage::GraphBegin(GraphBeginPayload {
        total_nodes: node_count,
        total_edges: edge_count,
    });
    write
        .send(Message::Text(serde_json::to_string(&begin)?))
        .await?;

    // Stream Nodes
    for chunk in nodes.chunks(50) {
        let batch = BroadcastMessage::NodeBatch(NodeBatchPayload {
            nodes: chunk.to_vec(),
        });
        write
            .send(Message::Text(serde_json::to_string(&batch)?))
            .await?;
    }
    info!("📤 Streamed {} nodes to {}", node_count, addr);

    // Stream Edges
    for chunk in edges.chunks(100) {
        let batch = BroadcastMessage::EdgeBatch(EdgeBatchPayload {
            edges: chunk.to_vec(),
        });
        write
            .send(Message::Text(serde_json::to_string(&batch)?))
            .await?;
    }
    info!("📤 Streamed {} edges to {}", edge_count, addr);

    // End Stream
    write
        .send(Message::Text(serde_json::to_string(
            &BroadcastMessage::GraphEnd,
        )?))
        .await?;
    info!("🏁 Graph stream complete for {}", addr);

    // Two-way message handling
    loop {
        tokio::select! {
            // Handle incoming messages from client
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        debug!("📥 Received from {}: {}", addr, text);
                        // Process client requests here (JSON-RPC)
                        // For now, just echo
                    }
                    Some(Ok(Message::Ping(data))) => {
                        write.send(Message::Pong(data)).await?;
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("👋 Client {} disconnected gracefully", addr);
                        break;
                    }
                    Some(Err(e)) => {
                        warn!("⚠️  Error from {}: {}", addr, e);
                        break;
                    }
                    None => break,
                    _ => {}
                }
            }

            // Forward broadcast messages to client
            msg = broadcast_rx.recv() => {
                match msg {
                    Ok(broadcast) => {
                        let json = serde_json::to_string(&broadcast)?;
                        if write.send(Message::Text(json)).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("Client {} lagged by {} messages", addr, n);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        }
    }

    info!("🔌 Connection closed: {}", addr);
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// File Watcher with Debouncing
// ─────────────────────────────────────────────────────────────────────────────

/// Runs the file watcher with debouncing.
async fn run_file_watcher(
    watch_path: PathBuf,
    extensions: Vec<String>,
    debounce_ms: u64,
    tx: mpsc::Sender<WatcherEvent>,
) -> notify::Result<()> {
    let (notify_tx, mut notify_rx) = mpsc::channel::<notify::Result<Event>>(256);

    // Create watcher in sync context
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = notify_tx.blocking_send(res);
        },
        Config::default(),
    )?;

    watcher.watch(&watch_path, RecursiveMode::Recursive)?;
    info!("👁️  File watcher started for {}", watch_path.display());

    // Debounce state
    let mut pending: HashMap<PathBuf, Instant> = HashMap::new();
    let debounce_dur = Duration::from_millis(debounce_ms);

    loop {
        // Process pending debounced events
        let now = Instant::now();
        let mut ready: Vec<PathBuf> = Vec::new();

        for (path, time) in pending.iter() {
            if now.duration_since(*time) >= debounce_dur {
                ready.push(path.clone());
            }
        }

        for path in ready {
            pending.remove(&path);
            if should_process_file(&path, &extensions) {
                let event = if path.exists() {
                    WatcherEvent::Changed(path)
                } else {
                    WatcherEvent::Deleted(path)
                };
                let _ = tx.send(event).await;
            }
        }

        // Wait for new events with timeout
        match tokio::time::timeout(Duration::from_millis(50), notify_rx.recv()).await {
            Ok(Some(Ok(event))) => {
                for path in event.paths {
                    if should_process_file(&path, &extensions) {
                        pending.insert(path, Instant::now());
                    }
                }
            }
            Ok(Some(Err(e))) => {
                warn!("Watch error: {}", e);
            }
            Ok(None) => break, // Channel closed
            Err(_) => {}       // Timeout, continue
        }
    }

    Ok(())
}

/// Checks if a file should be processed based on extension.
fn should_process_file(path: &Path, extensions: &[String]) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| extensions.iter().any(|e| e == ext))
        .unwrap_or(false)
}

// ─────────────────────────────────────────────────────────────────────────────
// Background Indexer
// ─────────────────────────────────────────────────────────────────────────────

/// Runs the background indexer that processes file changes.
async fn run_background_indexer(
    mut rx: mpsc::Receiver<WatcherEvent>,
    graph: SharedGraph,
    broadcast_tx: broadcast::Sender<BroadcastMessage>,
    _root_path: PathBuf,
) {
    let mut parser = ArborParser::new().expect("Failed to initialize parser");

    info!("🔧 Background indexer started");

    while let Some(event) = rx.recv().await {
        let start = Instant::now();

        match event {
            WatcherEvent::Changed(path) | WatcherEvent::Created(path) => {
                let file_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");

                info!("📝 Re-indexing: {}", file_name);

                match parser.parse_file(&path) {
                    Ok(result) => {
                        let mut g = graph.write().await;

                        // Remove old nodes from this file
                        g.remove_file(&result.file_path);

                        // Add new nodes
                        let mut node_ids = HashMap::new();
                        for symbol in &result.symbols {
                            let id = g.add_node(symbol.clone());
                            node_ids.insert(symbol.id.clone(), id);
                        }

                        // Add edges for relations
                        for relation in &result.relations {
                            if let Some(&from_id) = node_ids.get(&relation.from_id) {
                                // Try to find the target by name
                                let targets = g.find_by_name(&relation.to_name);
                                if let Some(target) = targets.first() {
                                    if let Some(to_id) = g.get_index(&target.id) {
                                        let edge_kind = match relation.kind {
                                            arbor_core::RelationType::Calls => EdgeKind::Calls,
                                            arbor_core::RelationType::Imports => EdgeKind::Imports,
                                            arbor_core::RelationType::Extends => EdgeKind::Extends,
                                            arbor_core::RelationType::Implements => {
                                                EdgeKind::Implements
                                            }
                                        };
                                        g.add_edge(from_id, to_id, Edge::new(edge_kind));
                                    }
                                }
                            }
                        }

                        let elapsed = start.elapsed();
                        info!(
                            "✅ Indexed {} in {:?} ({} symbols, {} relations)",
                            file_name,
                            elapsed,
                            result.symbols.len(),
                            result.relations.len()
                        );

                        // Broadcast update
                        let update = BroadcastMessage::GraphUpdate(GraphUpdatePayload {
                            is_delta: true,
                            node_count: g.node_count(),
                            edge_count: g.edge_count(),
                            file_count: g.stats().files,
                            changed_files: vec![result.file_path],
                            timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_secs(),
                            nodes: Some(g.nodes().cloned().collect()),
                            edges: Some(g.export_edges()),
                        });

                        let _ = broadcast_tx.send(update);
                    }
                    Err(e) => {
                        warn!("⚠️  Parse error for {}: {}", file_name, e);
                    }
                }
            }

            WatcherEvent::Deleted(path) => {
                let file_str = path.to_string_lossy().to_string();
                info!("🗑️  File deleted: {}", path.display());

                let mut g = graph.write().await;
                g.remove_file(&file_str);

                let update = BroadcastMessage::GraphUpdate(GraphUpdatePayload {
                    is_delta: true,
                    node_count: g.node_count(),
                    edge_count: g.edge_count(),
                    file_count: g.stats().files,
                    changed_files: vec![file_str],
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                    nodes: Some(g.nodes().cloned().collect()),
                    edges: Some(g.export_edges()),
                });

                let _ = broadcast_tx.send(update);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_process_file() {
        let extensions = vec!["ts".to_string(), "rs".to_string()];

        assert!(should_process_file(Path::new("foo.ts"), &extensions));
        assert!(should_process_file(Path::new("bar.rs"), &extensions));
        assert!(!should_process_file(Path::new("baz.py"), &extensions));
        assert!(!should_process_file(Path::new("README.md"), &extensions));
    }

    #[test]
    fn test_broadcast_message_serialization() {
        let msg = BroadcastMessage::GraphUpdate(GraphUpdatePayload {
            is_delta: true,
            node_count: 42,
            edge_count: 100,
            file_count: 5,
            changed_files: vec!["foo.ts".to_string()],
            timestamp: 1234567890,
            nodes: None,
            edges: None,
        });

        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("GraphUpdate"));
        assert!(json.contains("42"));
    }
}
