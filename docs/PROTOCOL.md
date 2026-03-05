# Arbor Protocol Specification

Version: 1.0.0

## Overview

The Arbor Protocol is a JSON-RPC 2.0 interface over WebSocket that allows AI agents and IDE integrations to query the code graph for architectural context.

## Connection

Default endpoint: `ws://localhost:7433`

The server supports multiple concurrent connections. Each connection maintains its own query state but shares the underlying graph.

## Message Format

All messages follow JSON-RPC 2.0:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "methodName",
  "params": { ... }
}
```

Responses:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": { ... }
}
```

## Methods

### `graph.info`

Returns metadata about the indexed graph.

**Request:**

```json
{
  "method": "graph.info",
  "params": {}
}
```

**Response:**

```json
{
  "result": {
    "nodeCount": 1542,
    "edgeCount": 4820,
    "languages": ["typescript", "rust", "python"],
    "lastIndexed": "2024-01-15T10:30:00Z",
    "version": "0.1.0"
  }
}
```

### `discover`

Finds the architectural root for a semantic query. Unlike simple text search, this traces the call graph to find the primary service or component.

**Request:**

```json
{
  "method": "discover",
  "params": {
    "query": "user authentication",
    "limit": 5
  }
}
```

**Response:**

```json
{
  "result": {
    "nodes": [
      {
        "id": "auth_controller_validate",
        "name": "AuthController.validate",
        "kind": "function",
        "file": "src/controllers/auth.ts",
        "qualified_name": "auth::AuthController::validate",
        "line_start": 45,
        "line_end": 62,
        "signature": "fn validate(token: &str) -> bool",
        "centrality": 0.95
      }
    ],
    "queryTime": 8
  }
}
```

### `impact`

Analyzes the blast radius of a code change. Returns all nodes that depend on the specified target.

**Request:**

```json
{
  "method": "impact",
  "params": {
    "node": "UserService.validateToken",
    "depth": 3
  }
}
```

**Response:**

```json
{
  "result": {
    "target": {
      "id": "user_service_validate_token",
      "name": "UserService.validateToken",
      "kind": "function",
      "line_start": 127,
      "line_end": 146,
      "qualified_name": "services::UserService::validateToken",
      "signature": "async validateToken(token: string): Promise<boolean>",
      "centrality": 0.76
      "line": 127
    },
    "dependents": [
        "node": {
          "id": "auth_middleware_check",
          "name": "authMiddleware.check",
          "qualified_name": "middleware::authMiddleware::check",
          "kind": "function",
          "file": "src/middleware/auth.ts",
          "line_start": 23,
          "line_end": 41,
          "signature": "async check(ctx: RequestContext)",
          "centrality": 0.62
        },
        "file": "src/middleware/auth.ts",
        "line": 23,
        "relationship": "calls",
        "depth": 1
      }
    ],
    "totalAffected": 47,
    "queryTime": 15
  }
}
```

### `context`

Retrieves ranked context for a task. Nodes are ordered by architectural significance, optimized for AI context windows.

**Request:**

```json
{
  "method": "context",
  "params": {
    "task": "refactor the payment processing flow",
    "maxTokens": 8000,
    "includeSource": true
  }
}
```

**Response:**

```json
{
  "result": {
    "nodes": [
      {
        "id": "payment_service",
        "name": "PaymentService",
        "qualified_name": "services::PaymentService",
        "kind": "class",
        "file": "src/services/payment.ts",
        "line_start": 15,
        "line_end": 245,
        "signature": null,
        "centrality": 0.87,
        "query_note": "Source embedding is client-side; protocol returns ranked node metadata"
      }
    ],
    "totalTokens": 7840,
    "queryTime": 22
  }
}
```

### `node.get`

Retrieves detailed information about a specific node.

**Request:**

```json
{
  "method": "node.get",
  "params": {
    "id": "payment_service_process"
  }
}
```

**Response:**

```json
{
  "result": {
    "id": "payment_service_process",
    "name": "PaymentService.process",
    "kind": "function",
    "file": "src/services/payment.ts",
    "lineStart": 67,
    "lineEnd": 125,
    "signature": "async process(order: Order): Promise<PaymentResult>",
    "edges": {
      "calledBy": ["checkout_controller_submit"],
      "calls": ["stripe_client_charge", "order_service_update"],
      "imports": ["stripe", "order_types"]
    }
  }
}
```

### `search`

Simple text search across node names and signatures.

**Request:**

```json
{
  "method": "search",
  "params": {
    "query": "validate",
    "kind": "function",
    "limit": 20
  }
}
```

**Response:**

```json
{
  "result": {
    "nodes": [
      {
        "id": "user_service_validate",
        "name": "UserService.validate",
        "qualified_name": "services::UserService::validate",
        "kind": "function",
        "file": "src/services/user.ts",
        "line_start": 45,
        "line_end": 57,
        "signature": "fn validate(input: &UserInput)",
        "centrality": 0.41
      }
    ],
    "total": 127,
    "queryTime": 3
  }
}
```

## Node Kinds

| Kind | Description |
|------|-------------|
| `function` | Standalone function or method |
| `class` | Class definition |
| `interface` | Type interface or protocol |
| `variable` | Module-level variable or constant |
| `import` | Import statement |
| `export` | Export declaration |
| `module` | File/module boundary |

## Edge Types

| Type | Description |
|------|-------------|
| `calls` | Function A calls function B |
| `calledBy` | Inverse of calls |
| `imports` | Module A imports from module B |
| `implements` | Class implements interface |
| `extends` | Class extends another class |
| `references` | General reference to a symbol |

## Error Codes

| Code | Message | Description |
|------|---------|-------------|
| -32700 | Parse error | Invalid JSON |
| -32600 | Invalid request | Missing required fields |
| -32601 | Method not found | Unknown method name |
| -32602 | Invalid params | Missing or invalid parameters |
| -32000 | Graph not ready | Index not yet complete |
| -32001 | Node not found | Requested node doesn't exist |

## Subscriptions

The protocol supports subscriptions for real-time updates:

### `graph.subscribe`

Subscribe to graph changes.

**Request:**

```json
{
  "method": "graph.subscribe",
  "params": {
    "events": ["nodeAdded", "nodeRemoved", "edgeAdded"]
  }
}
```

**Notification:**

```json
{
  "method": "graph.event",
  "params": {
    "type": "nodeAdded",
    "node": { ... }
  }
}
```

## Spotlight Protocol

The Spotlight Protocol enables real-time synchronization between AI agents, the Arbor Visualizer, and local development environments.

### Transport

| Property | Value |
|----------|-------|
| **Endpoint** | `ws://127.0.0.1:8081` (SyncServer) |
| **Latency Target** | <50ms for UI triggers |
| **Highlight System** | Golden Highlight (`#FFD700`) |

### Message: `FocusNode`

Broadcasts when an AI tool focuses on a specific node.

**Payload:**

```json
{
  "type": "FocusNode",
  "payload": {
    "node_id": "auth_controller_validate",
    "file": "src/controllers/auth.rs",
    "line": 45
  }
}
```

### Arbor Visualizer (Flutter)

| Component | Specification |
|-----------|--------------|
| **AnimationController** | 600ms duration |
| **Easing** | `easeOutCubic` |
| **Haptics** | Medium impact on arrival |
| **State** | `spotlightNodeId` in `GraphState` |

### VS Code Extension

| Feature | Behavior |
|---------|----------|
| **Spotlight Sync** | Highlights line when AI focuses |
| **Keyboard Shortcut** | `Ctrl+Shift+A` (Toggle Visualizer) |
| **Context Menu** | "Show in Arbor Visualizer" |
| **Status Bar** | Connection indicator |

### Centrality Metric (Impact Level)

Uses PageRank algorithm to determine symbol importance.

| Range | Classification |
|-------|---------------|
| > 0.8 | High Impact (heavily imported/referenced) |
| 0.3-0.8 | Medium Impact |
| < 0.3 | Low Impact (leaf nodes) |
