# Graph Schema

This document describes the data model used by Arbor to represent code structure.

## Nodes

Every code entity is represented as a node in the graph.

### Node Structure

```json
{
  "id": "unique_node_identifier",
  "name": "FunctionName",
  "qualifiedName": "ModuleName.ClassName.FunctionName",
  "kind": "function",
  "file": "src/services/user.ts",
  "lineStart": 45,
  "lineEnd": 78,
  "column": 2,
  "signature": "async validateUser(id: string): Promise<User>",
  "visibility": "public",
  "attributes": {
    "async": true,
    "static": false,
    "exported": true
  },
  "docstring": "Validates a user by their ID.",
  "centrality": 0.75
}
```

### Node Kinds

| Kind | Description | Languages |
|------|-------------|-----------|
| `function` | Standalone function | All |
| `method` | Class method | All |
| `class` | Class definition | All |
| `interface` | Interface/protocol/trait | TS, Rust |
| `struct` | Struct definition | Rust |
| `enum` | Enum definition | All |
| `variable` | Module-level variable | All |
| `constant` | Constant definition | All |
| `type_alias` | Type alias | TS, Rust |
| `module` | File/module boundary | All |
| `import` | Import statement | All |
| `export` | Export declaration | TS |

### Node IDs

Node IDs are generated deterministically from:

- File path (relative to project root)
- Node qualified name
- Node kind

This ensures the same node always gets the same ID, enabling incremental updates.

```
id = hash(file_path + ":" + qualified_name + ":" + kind)
```

## Edges

Edges represent relationships between nodes.

### Edge Structure

```json
{
  "from": "source_node_id",
  "to": "target_node_id",
  "kind": "calls",
  "location": {
    "file": "src/services/user.ts",
    "line": 52,
    "column": 8
  }
}
```

### Edge Kinds

| Kind | Description | From → To |
|------|-------------|-----------|
| `calls` | Function invocation | function → function |
| `imports` | Import statement | module → module |
| `exports` | Re-export | module → symbol |
| `extends` | Class inheritance | class → class |
| `implements` | Interface implementation | class → interface |
| `uses_type` | Type reference | any → type/interface |
| `references` | General symbol reference | any → any |
| `contains` | Nesting relationship | class → method |
| `returns` | Return type | function → type |
| `parameter` | Parameter type | function → type |

## Graph Structure

The graph is stored using an adjacency list representation:

```
nodes: HashMap<NodeId, Node>
edges_out: HashMap<NodeId, Vec<Edge>>
edges_in: HashMap<NodeId, Vec<Edge>>
```

Both incoming and outgoing edges are indexed for fast traversal in either direction.

## Indexes

For fast lookups, we maintain several indexes:

### Name Index

```
name_index: HashMap<String, Vec<NodeId>>
```

Maps symbol names to node IDs. Used for text search.

### File Index

```
file_index: HashMap<PathBuf, Vec<NodeId>>
```

Maps file paths to all nodes defined in that file. Used for incremental updates.

### Kind Index

```
kind_index: HashMap<NodeKind, Vec<NodeId>>
```

Maps node kinds to IDs. Used for filtering by type.

## Serialization

The graph can be serialized to JSON for export or persistence:

```json
{
  "version": "1.0",
  "projectRoot": "/path/to/project",
  "timestamp": "2024-01-15T10:30:00Z",
  "stats": {
    "nodeCount": 1542,
    "edgeCount": 4820
  },
  "nodes": [
    { "id": "...", "name": "...", ... }
  ],
  "edges": [
    { "from": "...", "to": "...", "kind": "..." }
  ]
}
```

## Language-Specific Mappings

### TypeScript

| AST Node | Arbor Kind |
|----------|------------|
| `function_declaration` | function |
| `method_definition` | method |
| `class_declaration` | class |
| `interface_declaration` | interface |
| `type_alias_declaration` | type_alias |
| `variable_declaration` | variable |
| `import_statement` | import |
| `export_statement` | export |

### Rust

| AST Node | Arbor Kind |
|----------|------------|
| `function_item` | function |
| `impl_item` → `function_item` | method |
| `struct_item` | struct |
| `enum_item` | enum |
| `trait_item` | interface |
| `use_declaration` | import |
| `mod_item` | module |

### Python

| AST Node | Arbor Kind |
|----------|------------|
| `function_definition` | function |
| `class_definition.function_definition` | method |
| `class_definition` | class |
| `import_statement` | import |
| `import_from_statement` | import |

## Centrality Algorithm

Arbor ranks nodes with a PageRank variant on the call graph:

1. Collapse every strongly connected component of `Calls` edges into one supernode. Cycles — closed rings and cycles that call out to a helper — are ranked as a component, then that mass is shared across the members.
2. Initialize component mass uniformly (`1/N` per member).
3. Iterate at most the caller's budget (early exit when the residual drops below `1e-9`). Damping is 0.85: 15% of the score teleports uniformly, and the rest follows external call edges. Callers in test files contribute 10% weight.
4. Report a percentile rank, `i / (n - 1)`, not the raw mass. `0.6` means "above 60% of this repository."

Nodes with high centrality are architecturally significant (many dependents) and are prioritized in context windows.
