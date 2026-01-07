## Description

**Arbor v1.0.0 "The Logic Forest"** - This release transforms Arbor into a production-grade Code Property Graph (CPG) engine. It introduces a persistent storage layer (`sled`), cross-file symbol resolution ("World Edges"), and a powerful AI bridge (`ArborQL` via MCP). It also adds support for C# and includes major performance optimizations for the visualizer.

## Type of Change

- [ ] Bug fix (non-breaking change that fixes an issue)
- [x] New feature (non-breaking change that adds functionality)
- [ ] Breaking change (fix or feature that would cause existing functionality to change)
- [x] Documentation update
- [x] Performance improvement
- [x] Code refactoring

## Changes Made

### 🧠 ArborQL & MCP (arbor-mcp/)

- Implemented **ArborQL** pathfinding (`find_path`) using A* algorithm to trace logic flows.
- Added `get_context` and `analyze_impact` tools for AI agents.
- Integrated Model Context Protocol (MCP) over stdio.

### 💾 Persistence Layer (arbor-graph/store.rs)

- Implemented **GraphStore** using `sled` database.
- **Incremental Sync**: Only writes "dirty" nodes to disk using atomic batches and a `f:{file} -> [n:{id}]` index.
- **Binary Serialization**: Uses `bincode` for high-performance state saving.

### 🔗 World Edges (arbor-graph/)

- Implemented **Global Symbol Table** for mapping Fully Qualified Names (FQNs) to Node IDs.
- Added **Cross-File Resolution** logic in `GraphBuilder` to link calls across file boundaries.

### ⚡ Visualizer Optimizations (visualizer/)

- **Velocity-Based LOD**: `GraphPainter` simplifies rendering during rapid panning/zooming to maintain 60 FPS.
- **Incremental Sled Sync**: Flutter client now benefits from the backend's efficient data loading.
- **Barnes-Hut QuadTree**: O(n log n) force layout for 100k+ nodes.

### 📝 Logic & Languages

- **C# Parser**: Added `tree-sitter-c-sharp` support.
- **Consolidated Parsers**: Refactored 10 language parsers into `parser_v2.rs`.
- **Edge Types**: Added `FlowsTo` (Control Flow) and `DataDependency` edges.

## Testing

- [x] Ran `cargo test --all` ✅ All tests passing (including `arbor-graph` persistence)
- [x] Manual Verification:
  - Validated `find_path` tool with MCP.
  - Verified persistence by restarting server and checking graph state.
  - Verified visualizer performance with large dataset.

## Checklist

- [x] My code follows the project's style guidelines
- [x] I have added tests for my changes
- [x] I have updated the documentation where necessary (CHANGELOG.md, README.md)
- [x] All new and existing tests pass
- [x] I have added appropriate comments where the code isn't self-explanatory
