<p align="center">
  <img src="docs/assets/arbor-logo.svg" alt="Arbor" width="120" height="120" />
</p>

# Arbor v1.4.0

**The Graph-Native Intelligence Layer for Code**

> Know what breaks before you break it.

---

<p align="center">
  <a href="https://github.com/Anandb71/arbor/actions"><img src="https://img.shields.io/github/actions/workflow/status/Anandb71/arbor/rust.yml?style=flat-square&label=CI" alt="CI" /></a>
  <img src="https://img.shields.io/badge/release-v1.4.0-blue?style=flat-square" alt="Release" />
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License" />
</p>

## Overview

Arbor is a local-first impact analysis engine for large codebases. Unlike traditional search or RAG which relies on keyword similarity, Arbor parses your code into a semantic graph, allowing you to trace actual execution paths and dependencies.

### Example: Blast Radius Detection

Before refactoring `detect_language`, see exactly what depends on it:

```bash
$ arbor refactor detect_language

Analyzing detect_language...

Confidence: High | Role: Core Logic
• 15 callers, 3 dependencies
• Well-connected with manageable impact

> 18 nodes affected (4 direct, 14 transitive)

Immediate Impact:
  • parse_file (function)
  • get_parser (function)

Recommendation: Proceed with caution. Verify affected callers.
```

---

## Graphical Interface

Arbor v1.4 includes a native GUI for visual impact analysis.

```bash
arbor gui
```

![Arbor GUI](docs/gui_screenshot.png)

**Key Capabilities:**
- **Symbol Search**: Instant lookup for functions, classes, and methods.
- **Impact Analysis**: Visualize direct and indirect dependencies.
- **Privacy**: File paths are hidden by default to prevent accidental leaks in screenshots.
- **Export**: Copy analysis results as Markdown for PR descriptions.

> **Note:** The CLI and GUI share the same analysis engine.

---

## Quick Start

1. **Install Arbor** (includes both CLI and GUI):
   ```bash
   cargo install arbor-graph-cli
   ```

2. **Run Impact Analysis**:
   ```bash
   cd your-project
   arbor refactor <function-name>
   ```

3. **Launch GUI**:
   ```bash
   arbor gui
   ```

> See the [Quickstart Guide](docs/QUICKSTART.md) for advanced commands.

---

## Why Arbor?

Most AI coding tools treat code as unstructured text, relying on vector similarity which lacks precision.

**Arbor builds a graph.** Every function, class, and import is a node; every call is an edge. When you ask "what breaks if I change this?", Arbor traces the actual execution path rather than guessing based on keyword matches.

```text
Traditional RAG:         Arbor Graph Analysis:

"auth" → 47 results      "auth" → AuthController
(keyword match)                   ├── calls → TokenMiddleware
                                  ├── queries → UserRepository
                                  └── emits → AuthEvent
```

## Features

### Native GUI
A lightweight, high-performance interface for impact analysis. Included securely in the main binary.

### Confidence Scoring
Every analysis provides an explainable confidence level:
- **High**: Well-connected, static resolution confirmed.
- **Medium**: Some uncertainty in resolution.
- **Low**: Relies on heuristics or involves dynamic dispatch.

### Node Classification
Nodes are automatically classified by their architectural role:
- **Entry Point**: API endpoints or main functions.
- **Core Logic**: Domain-specific business logic.
- **Utility**: Helper functions widely used across the codebase.
- **Adapter**: Interface layers and bridges.

### AI Bridge (MCP)
Arbor implements the Model Context Protocol (MCP), allowing LLMs (like Claude) to:
- `find_path(start, end)`: Discover logic flow between components.
- `analyze_impact(node)`: Determine blast radius programmatically.
- `get_context(node)`: Retrieve semantically linked code.

### Cross-File Resolution
Arbor resolves imports, class inheritance, and function calls across file boundaries using a global symbol table. It distinguishes between `User` in `auth.ts` and `User` in `types.ts`.

---

## Supported Languages

| Language   | Status | Parser Entity Coverage |
|------------|--------|------------------------|
| **Rust**       | ✅     | Functions, Structs, Impls, Traits, Macros |
| **TypeScript** | ✅     | Classes, Interfaces, Types, Imports, JSX |
| **JavaScript** | ✅     | Functions, Classes, Vars, Imports |
| **Python**     | ✅     | Classes, Functions, Imports, Decorators |
| **Go**         | ✅     | Structs, Interfaces, Funcs, Methods |
| **Java**       | ✅     | Classes, Interfaces, Methods, Fields, Connectors |
| **C**          | ✅     | Structs, Functions, Enums, Typedefs |
| **C++**        | ✅     | Classes, Namespaces, Templates, Impls |
| **C#**         | ✅     | Classes, Methods, Properties, Interfaces, Structs |
| **Dart**       | ✅     | Classes, Mixins, Methods, Widgets |

> **Python support:** Includes static analysis for decorators, `__init__.py` modules, and `@dataclass` patterns. Dynamic dispatch is marked as uncertain.

---

## Build from Source

```bash
git clone https://github.com/Anandb71/arbor.git
cd arbor/crates
cargo build --release
```

### Linux Dependencies
If building the GUI on Linux, install development headers:
```bash
sudo apt-get install -y pkg-config libx11-dev libxcb-shape0-dev libxcb-xfixes0-dev \
    libxkbcommon-dev libgtk-3-dev libfontconfig1-dev libasound2-dev libssl-dev cmake
```

---

## Troubleshooting

### Why was my symbol not found?
- **GitIgnore**: Arbor respects `.gitignore`. Check with `arbor status --files`.
- **Extension**: Ensure the file type is supported (e.g., `.rs`, `.ts`, `.py`).
- **Content**: Empty files are skipped (except `__init__.py`).
- **Dynamic Calls**: Purely dynamic calls (e.g., `eval`) may not be detected.
- **Typo**: Symbols are case-sensitive. Use `arbor query <partial_name>` to search.

### Graph is empty?
Run `arbor status` to verify file detection and parser health.

---

## Security

Arbor operates on a **Local-First** security model:
- **No Exfiltration**: All analysis happens locally on your machine.
- **Offline Capable**: No API keys or internet connection required.
- **Open Source**: Full transparency for security audits.

---

## License

MIT License. See [LICENSE](LICENSE) for details.

<p align="center">
  <a href="https://github.com/Anandb71/arbor">⭐ Star us on GitHub</a>
</p>
