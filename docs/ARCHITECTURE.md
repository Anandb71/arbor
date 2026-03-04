# Arbor Architecture

This document describes the high-level architecture of Arbor and how its components interact.

## System Overview: The Unified Nervous System

Arbor is a "Unified Nervous System" that connects your codebase, AI agents, and development environment.

```
                    ┌─────────────────────────────────────────────────────────┐
                    │              THE UNIFIED NERVOUS SYSTEM                 │
                    └─────────────────────────────────────────────────────────┘
                                              │
          ┌───────────────────────────────────┼───────────────────────────────────┐
          │                                   │                                   │
          ▼                                   ▼                                   ▼
   ┌─────────────┐                   ┌─────────────────┐                 ┌─────────────┐
   │   VS Code   │◄──── Spotlight ──►│  Arbor Bridge   │◄── Spotlight ──►│  Visualizer │
   │  Extension  │      Protocol     │  (MCP Server)   │     Protocol    │   (Flutter) │
   └─────────────┘                   └────────┬────────┘                 └─────────────┘
          │                                   │                                   │
          │ Golden Highlight                  │ Architectural Brief              │ Camera
          │ (#FFD700)                         │ (Markdown Tables)                │ Animation
          │                                   │                                   │
          └───────────────────────────────────┼───────────────────────────────────┘
                                              │
                                     ┌────────┴────────┐
                                     │   SyncServer    │
                                     │  (WebSocket)    │
                                     │  ws://8080      │
                                     └────────┬────────┘
                                              │
                    ┌─────────────────────────┼─────────────────────────┐
                    │                         │                         │
                    ▼                         ▼                         ▼
           ┌─────────────┐           ┌─────────────┐           ┌─────────────┐
           │arbor-server │           │ arbor-graph │           │arbor-watcher│
           │  (JSON-RPC) │           │ (petgraph)  │           │  (notify)   │
           └─────────────┘           └─────────────┘           └─────────────┘
                                              │
                                              ▼
                                     ┌─────────────────┐
                                     │   arbor-core    │
                                     │  (Tree-sitter)  │
                                     │   144ms parse   │
                                     └────────┬────────┘
                                              │
                                              ▼
                                     ┌─────────────────┐
                                     │    Codebase     │
                                     │  (Your Files)   │
                                     └─────────────────┘
```

### The Flow

1. **Parsing**: `arbor-core` parses your codebase with Tree-sitter (~144ms for 10k lines)
2. **Graphing**: `arbor-graph` builds a dependency graph with petgraph
3. **Watching**: `arbor-watcher` detects file changes and triggers re-indexing
4. **Serving**: `arbor-server` exposes the graph via JSON-RPC over WebSocket
5. **Syncing**: `SyncServer` broadcasts real-time updates to all clients
6. **Bridging**: `arbor-mcp` enables AI agents to query the graph
7. **Spotlighting**: When AI queries a node, the Spotlight Protocol broadcasts focus events
8. **Visualizing**: Flutter visualizer animates to the spotlighted node
9. **Highlighting**: VS Code extension highlights the corresponding line

## Crate Responsibilities

### arbor-core

The foundational crate that handles parsing source files into AST nodes.

**Key Components:**

- **Parser**: Wrapper around Tree-sitter that handles language detection and parsing
- **Language Registry**: Maps file extensions to language parsers
- **Node Extraction**: Traverses the AST to extract functions, classes, variables, etc.
- **Language Modules**: Per-language logic for TypeScript, Rust, Python

**Public API:**

```rust
pub fn parse_file(path: &Path) -> Result<Vec<CodeNode>, ParseError>;
pub fn detect_language(path: &Path) -> Option<Box<dyn LanguageParser>>;
```

### arbor-graph

Manages the in-memory graph of code relationships.

**Key Components:**

- **Graph**: Uses petgraph for efficient graph operations
- **Schema**: Defines NodeKind, EdgeType, and their attributes
- **Builder**: Constructs the graph from parsed code nodes
- **Query Engine**: Traversal, search, and filtering operations
- **Ranking**: Centrality scoring (simplified PageRank variant)

**Public API:**

```rust
pub struct ArborGraph {
    pub fn add_node(&mut self, node: CodeNode) -> NodeId;
    pub fn add_edge(&mut self, from: NodeId, to: NodeId, kind: EdgeKind);
    pub fn find_by_name(&self, name: &str) -> Vec<&CodeNode>;
    pub fn get_callers(&self, id: NodeId) -> Vec<&CodeNode>;
    pub fn get_callees(&self, id: NodeId) -> Vec<&CodeNode>;
    pub fn compute_centrality(&mut self);
}
```

### arbor-watcher

Handles file system watching and incremental updates.

**Key Components:**

- **Watcher**: Uses notify crate to watch for file changes
- **Delta Engine**: Determines what changed and what needs re-parsing
- **Cache**: Stores parsed AST for unchanged files
- **Debouncer**: Batches rapid file changes to avoid thrashing

**Design Notes:**

The watcher maintains a hash of each file's contents. When a file changes, it compares the new hash with the cached one. If different, it triggers a re-parse of only that file.

For sub-100ms updates, we use a two-tier cache:

1. **File-level cache**: Stores the full AST per file
2. **Node-level cache**: Stores individual nodes with their byte ranges

When a file changes, we first try a "surgical" update by re-parsing only the changed byte range. If that fails (structural change), we fall back to full file re-parse.

### arbor-server

WebSocket server implementing the Arbor Protocol.

**Key Components:**

- **Server**: Tokio-based async WebSocket server
- **Protocol Handler**: Parses JSON-RPC messages
- **Query Handlers**: Implements discover, impact, context, etc.
- **Subscription Manager**: Handles real-time event subscriptions

**Threading Model:**

The server runs on a Tokio runtime with:

- Main thread: Accepts connections
- Per-connection task: Handles messages for that client
- Background task: Runs the watcher and updates the graph

Graph access is protected by an RwLock. Reads are concurrent, writes are exclusive.

### arbor-cli

Command-line interface for end users.

**Commands:**

| Command | Description |
|---------|-------------|
| `arbor setup` | One-shot onboarding (initialize + index) |
| `arbor init` | Creates `.arbor/` config directory |
| `arbor index` | Full index of the codebase |
| `arbor query <q>` | Search the graph |
| `arbor serve` | Start the sidecar server |
| `arbor export` | Export graph to JSON |
| `arbor status` | Show index status |
| `arbor watch` | Re-index automatically on file changes |
| `arbor viz` | Launch the Logic Forest visualizer |
| `arbor bridge` | Start MCP server for AI integration |
| `arbor bridge --viz` | MCP + Visualizer together |
| `arbor doctor` (`check-health`) | System diagnostics and health check |
| `arbor refactor <symbol>` | Blast-radius preview before refactor |
| `arbor explain <symbol>` | Graph-backed context for AI/code reviews |
| `arbor audit <sink>` | Security-oriented path tracing to sensitive sinks |

### Visualizer Features (v0.1.0)

The Flutter-based Arbor Visualizer includes:

- **Follow Mode**: When enabled (default), the camera automatically animates to the node the AI is currently focusing on.
- **Low GPU Mode**: Toggle to disable bloom and blur effects for improved performance on lower-end hardware.

## Data Flow

### Initial Indexing

1. User runs `arbor index`
2. CLI recursively finds all source files (respecting .gitignore)
3. For each file:
   - Detect language from extension
   - Parse with Tree-sitter
   - Extract code nodes (functions, classes, etc.)
   - Add nodes to graph
4. Second pass: resolve edges (calls, imports, etc.)
5. Compute centrality scores
6. Write graph to `.arbor/graph.json`

### Incremental Update

1. Watcher detects file change
2. Debouncer waits for additional changes (50ms window)
3. Delta engine determines affected files
4. For each affected file:
   - Remove old nodes from graph
   - Re-parse file
   - Add new nodes to graph
5. Re-compute edges for affected nodes
6. Update centrality (incremental approximation)

### Query Handling

1. Client sends WebSocket message
2. Protocol handler parses JSON-RPC
3. Query router dispatches to appropriate handler
4. Handler queries the graph (read lock)
5. Results are ranked and formatted
6. JSON response sent back to client

## Performance Considerations

### Memory

- Graph is stored in-memory using petgraph with adjacency list representation
- Nodes store minimal data (id, name, kind, location)
- Full source code is not stored; it's read from disk on demand
- Estimated: ~1KB per node, so 100k nodes ≈ 100MB

### Speed

- Tree-sitter is extremely fast (~100k lines/second)
- Graph operations are O(1) for lookups, O(n) for traversals
- Centrality is computed incrementally after initial full computation
- Debouncing prevents thrashing on rapid file changes

### Concurrency

- RwLock allows concurrent reads (queries from multiple clients)
- Writes (graph updates) are batched and infrequent
- Tokio provides efficient async I/O for WebSocket handling

## Error Handling

Each crate defines its own error type that implements `std::error::Error`. Errors are propagated up the stack and converted to appropriate JSON-RPC error codes at the protocol layer.

```rust
// In arbor-core
pub enum ParseError {
    IoError(std::io::Error),
    UnsupportedLanguage(String),
    TreeSitterError(String),
}

// In arbor-server, converted to:
{
  "error": {
    "code": -32602,
    "message": "Unsupported language: xyz"
  }
}
```
