# Changelog

## [Unreleased]

### Added

- None yet.

### Changed

- None yet.

## [1.7.0] - 2026-03-25 "Distribution & Reach"

> **Feature release focused on making Arbor available everywhere — every package manager, every editor, every CI pipeline.**

### Added

- **Automated release workflow** (`release.yml`) — Cross-platform binary builds (5 targets), crates.io publishing, and GitHub Release creation on tag push
- **Homebrew formula** (`packaging/homebrew/arbor.rb`) — macOS/Linux install via `brew install`
- **Scoop manifest** (`packaging/scoop/arbor.json`) — Windows install via `scoop install`
- **npm wrapper** (`packaging/npm/`) — Cross-platform install via `npx @arbor-graph/cli`
- **VS Code extension: 5 new commands** — `arbor.refactor`, `arbor.status`, `arbor.quickPick`, `arbor.diff`, `arbor.index`
- **VS Code extension: Quick-pick command menu** — `Ctrl+Shift+R` for all Arbor actions
- **VS Code extension: Walkthrough onboarding** — Get Started guide with step-by-step setup
- **VS Code extension: New settings** — `arbor.autoIndex`, `arbor.maxBlastRadius`
- **GitHub Sponsors** (`.github/FUNDING.yml`)
- **Academic citation** (`CITATION.cff`)
- **Docker Compose bridge service** for MCP container usage

### Changed

- **Dockerfile** updated to Rust 1.85 with OCI labels, git support
- **docker-compose.yml** modernized (removed deprecated `version` key)
- **VS Code extension categories** improved for marketplace discoverability
- **README badges** expanded (crates.io, GitHub Release, GHCR, Docker)
- **Install instructions** expanded with Homebrew, Scoop, npm, Docker options

### Removed

- **`vscode-publish.yml`** workflow (duplicate of `vscode-marketplace.yml`, caused CI failures)
- Stale `package-lock.json`, `crates/Cargo.lock`, `crates/test_output.txt`

## [1.6.2] - 2026-03-24 "Revival Release: Language Expansion + Developer Momentum"

> **Feature release focused on expanding parser reach, improving live sync coverage, and strengthening release momentum workflows.**

### Added

- **Fallback parser engine** in `arbor-core` for rapid support of additional ecosystems when a full Tree-sitter path is unavailable in all runtime surfaces
- **New language extension support (5+)** via fallback parsing:
  - Kotlin (`.kt`, `.kts`)
  - Swift (`.swift`)
  - Ruby (`.rb`)
  - PHP (`.php`, `.phtml`)
  - Shell (`.sh`, `.bash`, `.zsh`)
- **Regression tests** for fallback parsing in both legacy parser path and query parser v2 path

### Changed

- **Indexer support matrix** now includes fallback-language extensions in support checks
- **Bridge + visualizer sync watchers** now watch and re-index the newly added language extensions
- **CLI empty-graph hints** now include the expanded extension set
- **Workspace crate line bumped** to `1.6.2` and internal crate dependency versions aligned

### Documentation

- Updated release/status messaging and supported-language listings
- Added release notes for `v1.6.2`

## [1.6.1.1] - 2026-03-18 "Maintenance + Ecosystem Alignment"

> **Maintenance release focused on workflow reliability, MCP guidance, and ecosystem currency as of March 18, 2026.**

### Added

- **CLI: `arbor diff`** — Git-aware blast radius preview for changed files
  - Handles rename-aware changed-file detection
  - Ignores whitespace-only diffs
  - Filters generated/internal files for cleaner signal
- **CLI: `arbor check`** — CI-oriented risk gate over changed blast radius
  - Supports machine-readable JSON output for automation
- **CLI: `arbor open <symbol>`** — Opens symbol/file location in configured editor
- **CLI: `arbor index --changed-only`** — Incremental re-index path based on git changes
- **Binary graph snapshots** — `.arbor/graph.bin` read/write support for faster warm starts
- **Integration tests for diff heuristics** — rename, whitespace-only, generated-file scenarios
- **Workspace cleanup scripts** — `scripts/clean.ps1` and `scripts/clean.sh` to safely prune large generated artifacts before releases

### Changed

- **Branching guidance** documented for `main`, `release/v1.5`, and `release/v1.6`
- **Documentation refresh** across README, Quickstart, Install, Architecture, and MCP integration guides
- **Troubleshooting guidance** now includes a dedicated workflow for reclaiming multi-GB workspace bloat

### Maintenance

- Release channel and status messaging aligned to the `1.6.1.1` maintenance cut.
- Workspace crate version advanced to `1.6.1` (SemVer-compliant crate line for Cargo).
- MCP server metadata version now reports `1.6.1.1` for client-visible maintenance tracking.
- Release context refreshed against current ecosystem signals (Rust `1.94.0`, tree-sitter `0.26.7`, and broader MCP client/platform adoption).

### Documentation

- Added formal release notes for v1.6.0 in `docs/RELEASE_NOTES_v1.6.0.md`
- Added formal release notes for v1.6.1.1 in `docs/RELEASE_NOTES_v1.6.1.1.md`

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
