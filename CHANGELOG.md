# Changelog

## [1.1.0] - 2026-01-08 "The Sentinel Update"

> **Predict breakage. Give AI only the logic it needs.**

### Added

- **Impact Radius Simulator** (`impact.rs`) — Bidirectional BFS to predict all affected nodes before refactoring
  - Severity classification: direct (1 hop), transitive (2-3), distant (4+)
  - Entry edge tracking for explainability
  - Stable ordering for reproducible output
  - 8 unit tests including cycle detection
- **Dynamic Context Slicing** (`slice.rs`) — Token-bounded context extraction for LLM prompts
  - Pinning support for critical nodes
  - Explicit truncation reasons (budget vs depth)
  - 6 unit tests
- **MCP `analyze_impact` Tool** — Structured JSON output for AI agents
  - Input: `{ "node_id": "...", "max_depth": 5 }`
  - Returns: target, upstream, downstream, severity, hop_distance, entry_edge
- **CLI: `arbor refactor <target>`** — Preview blast radius before making changes
  - `--why` flag shows reasoning for each affected node
  - `--json` flag for scripting and CI integration
  - `--depth N` controls search depth
- **CLI: `arbor explain <target>`** — Graph-backed context for code explanations
  - `--why` flag shows path traced
  - `--json` flag for structured output
  - `--tokens N` controls context budget

### Changed

- MCP `analyze_impact` now uses real graph traversal (was placeholder)

---

All notable changes to Arbor will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] - 2026-01-07

### Added

- **World Edges (Cross-File Resolution)** - Implemented `SymbolTable` and FQN-based linking for robust cross-file references.
- **Persistence Layer** - Integrated `sled` database for local graph storage (`GraphStore`).
- **ArborQL (MCP)** - Added `find_path` tool for finding shortest paths between nodes.
- **C# language support** - Methods, classes, interfaces, structs, constructors, properties
- **Control Flow edges** - `FlowsTo` edge kind for CFG (Control Flow Graph) analysis
- **Data Flow edges** - `DataDependency` edge kind for DFA (Data Flow Analysis)
- **Barnes-Hut QuadTree** - O(n log n) force simulation for visualizer scalability
- **Viewport culling** - Only render visible nodes/edges for 100k+ node support
- **LOD rendering** - Simplified node rendering at low zoom levels
- **Headless mode** - `--headless` CLI flag for remote/Docker/WSL deployment
- **Binary serialization** - `bincode` dependency for future binary wire protocol

### Changed

- Consolidated language parsers into query-based `parser_v2.rs`
- Upgraded supported languages to 10 (TypeScript, JavaScript, Rust, Python, Go, Java, C, C++, Dart, C#)
- Improved graph rendering performance for large codebases

### Fixed

- None

## [0.1.1] - 2026-01-06

### Added

- **Go language support** - Functions, methods, structs, interfaces, imports
- **Java language support** - Classes, interfaces, methods, constructors, fields
- **C language support** - Functions, structs, enums, typedefs, includes
- **C++ language support** - Classes, namespaces, structs, functions, templates
- **Dart language support** - Classes, mixins, extensions, methods, enums
- `Constructor` and `Field` node kinds for Java/OOP languages
- Updated set-topics workflow with 19 repository topics

### Changed

- Expanded supported languages from 4 to 9
- Updated README with new language support table

### Fixed

- None

## [0.1.0] - 2026-01-05

### Added

- Initial release
- Core AST parsing with tree-sitter
- TypeScript/JavaScript language support
- Rust language support
- Python language support
- Interactive force-directed graph visualizer (Flutter)
- WebSocket-based real-time updates
- MCP (Model Context Protocol) bridge for AI agents
- CLI with `parse`, `graph`, and `bridge` commands
- File watching with hot reload
