<p align="center">
  <img src="docs/assets/arbor-logo.svg" alt="Arbor" width="120" height="120" />
</p>

<h1 align="center">Arbor v1.0.0</h1>

<p align="center">
  <strong>The Graph-Native Intelligence Layer for Code</strong><br>
  <em>Stop RAG-ing. Start navigating.</em>
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> •
  <a href="#why-arbor">Why Arbor?</a> •
  <a href="#features">Features</a> •
  <a href="#the-unified-nervous-system">Architecture</a> •
  <a href="docs/PROTOCOL.md">Protocol</a> •
  <a href="CONTRIBUTING.md">Contributing</a>
</p>

<p align="center">
  <a href="https://github.com/Anandb71/arbor/actions"><img src="https://img.shields.io/github/actions/workflow/status/Anandb71/arbor/rust.yml?style=flat-square&label=CI" alt="CI" /></a>
  <img src="https://img.shields.io/badge/release-v1.0.0-blue?style=flat-square" alt="Release" />
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License" />
  <img src="https://img.shields.io/badge/rust-1.70+-orange?style=flat-square" alt="Rust" />
  <img src="https://img.shields.io/badge/flutter-3.0+-blue?style=flat-square" alt="Flutter" />
  <img src="https://img.shields.io/badge/parse-144ms-gold?style=flat-square" alt="144ms Parse" />
  <a href="https://glama.ai/mcp/servers/Anandb71/arbor"><img src="https://glama.ai/mcp/servers/Anandb71/arbor/badge" alt="Glama MCP Server" /></a>
</p>

---

> **If you've ever asked an AI to refactor code and it confidently broke half your system, Arbor is why that keeps happening — and how to fix it.**

## Why Arbor?

**Arbor is designed to replace embedding-based RAG for code.**

Most AI coding assistants treat your codebase like a bag of text. They embed chunks into vectors and hope similarity search finds the right context. The result? Hallucinated connections, missing dependencies, and refactors that break everything downstream.

**Arbor is the ground truth layer between code and AI.**

We parse your code into an AST using [Tree-sitter](https://tree-sitter.github.io/), then build a living graph where every function, class, and variable is a **node**, and every import, call, and implementation is an **edge**. When an AI asks "where is authentication handled?", Arbor traces the call graph to find the actual service — not 47 keyword matches.

```text
Traditional RAG:         Arbor:
                         
"auth" → 47 results      "auth" → AuthController
                                  ├── validates via → TokenMiddleware  
                                  ├── queries → UserRepository
                                  └── emits → AuthEvent
```

## Quick Start

> 📖 **New to Arbor?** See the [5-minute Quickstart Guide](docs/QUICKSTART.md)

### Option 1: Download Pre-built Binary (Recommended)

Download `arbor-windows-v1.0.0.zip` from the [Releases](https://github.com/Anandb71/arbor/releases) page.

```bash
# Unzip and add to PATH, then:
cd your-project
arbor init
arbor index
arbor bridge --viz   # Starts server + opens visualizer
```

### Option 2: Build from Source

```bash
# Clone and build
git clone https://github.com/Anandb71/arbor.git
cd arbor/crates
cargo build --release

# Build visualizer (requires Flutter)
cd ../visualizer
flutter build windows
```

That's it. Your IDE or AI agent can now connect to `ws://localhost:7433` and query the graph, or use the MCP protocol over stdio.

## Features

### 🧠 ArborQL & AI Bridge (MCP)

Arbor enables Claude and other LLMs to "walk" your code graph. Using the Model Context Protocol (MCP), agents can run:

- `find_path(start, end)`: Discover the logic flow between two distant components (A* algorithm).
- `analyze_impact(node)`: Determine the blast radius of a change before it happens.
- `get_context(node)`: Retrieve semantically relevant code, not just keyword matches.

### 🔗 World Edges (Cross-File Resolution)

Arbor understands that code doesn't live in isolation. It resolves **imports**, **calls**, and **inheritances** across file boundaries using a Global Symbol Table.

**Concrete example:** When you import `User` in `profile.ts`, Arbor knows it's the same `User` class defined in `auth.ts`. If you rename `User.email` → `User.emailAddress`, Arbor can tell you which 7 files in `services/` will break — before you run the tests.

### 💾 Incremental Persistence

Powered by **Sled**, Arbor's graph persistence layer is atomicity-compliant and lightning fast.

- **Granular Updates**: Only "dirty" nodes are re-written to disk. Saving a file in a 100k LOC repo triggers minimal I/O.
- **Instant Load**: The graph state loads instantly on startup, no re-indexing required.

### 🌲 Logic Forest Visualizer (Debugging & Trust)

The visualizer exists to make AI reasoning **inspectable**. Every node an LLM touches can be seen, traced, and verified by a human.

- **Force-Directed Graph**: 100k+ nodes with Barnes-Hut QuadTree optimization
- **AI Spotlight**: Camera follows the node an AI agent is currently examining
- **Impact Highlights**: See which nodes "vibrate" when you preview a change
- **Interactive**: Drag nodes, hover for details, filter by file/type

![Arbor Visualizer](docs/assets/visualizer-screenshot.png)

### ⚡ Performance

- **Sub-100ms Incremental Sync**: Parsing happens in milliseconds.
- **Binary Serialization**: Graph state uses efficient `bincode` encoding.
- **Rust Core**: Built on the safety and speed of Rust.

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

## Platform Support

| Platform | CLI | Visualizer |
|----------|-----|------------|
| **Windows** | ✅ | ✅ |
| **macOS** | ✅ | ✅ |
| **Linux** | ✅ | ✅ |

### Monorepo & Symlink Support

For monorepos using `pnpm`, `npm link`, or symlinked local packages:

```bash
arbor index --follow-symlinks
arbor viz --follow-symlinks
arbor bridge --follow-symlinks
```

This ensures symlinked dependencies are indexed instead of skipped.

## Project Structure

```
arbor/
├── crates/                 # Rust workspace
│   ├── arbor-core/         # AST parsing, Tree-sitter integration
│   ├── arbor-graph/        # Graph schema, Sled Store, Symbol Table
│   ├── arbor-watcher/      # File watching, incremental sync
│   ├── arbor-server/       # WebSocket server, protocol handler
│   ├── arbor-mcp/          # Model Context Protocol bridge
│   └── arbor-cli/          # Command-line interface (pkg: arbor-graph-cli)
├── visualizer/             # Flutter desktop app
│   ├── lib/
│   │   ├── core/           # Theme, state management
│   │   ├── graph/          # Force-directed layout, LOD logic
│   │   └── shaders/        # GLSL bloom/glow effects
│   └── shaders/            # Raw GLSL files
└── docs/                   # Extended documentation
```

## Roadmap

### v1.0.0 (Completed)

- [x] **Phase 1**: Core indexer and CLI
- [x] **Phase 2**: Logic Forest visualizer (LOD, Bloom)
- [x] **Phase 3**: VS Code extension
- [x] **Phase 4**: Agentic Bridge (MCP)
- [x] **Phase 5**: Linux ARM64/AMD64 + macOS ARM64 CI/CD
- [x] **Phase 6**: Language server protocol support
- [x] **Phase 7**: Go and Java parser support
- [x] **Phase 8**: C/C++ parser support
- [x] **Phase 9**: Dart/Flutter parser support
- [x] **Phase 10**: The Brain Upgrade (Control Flow & Data Flow)
- [x] **Phase 11**: Expanded Support (C# Parser, Graph Persistence)

### v1.1.0 "The Sentinel Update" ✅

> **Predict breakage. Give AI only the logic it needs.**

- [x] **Impact Radius Simulator**: Bidirectional BFS blast radius analysis
- [x] **Dynamic Context Slicing**: Token-bounded LLM context with pinning
- [x] **CLI: `arbor refactor`** with `--why` and `--json` flags
- [x] **CLI: `arbor explain`** for graph-backed code explanation

```bash
$ arbor refactor validate --why

⚠️  Blast Radius
Target: validate (Function)

Total: 12 nodes (direct: 4, transitive: 6, distant: 2)

Direct (1 hop):
  • TokenMiddleware (Class) — Calls validate
  • AuthController (Class) — Calls validate
```

📖 **Full roadmap**: [docs/ROADMAP.md](docs/ROADMAP.md)

## Security

Arbor is designed with a **Local-First** security model:

- **No data exfiltration**: All indexing and querying happens 100% locally. No code leaves your machine.
- **No API keys required**: Works entirely offline.
- **No telemetry**: Zero phone-home behavior.
- **Open source**: Full source code available for audit.

## Who Arbor Is For

**Arbor is for:**

- Large or long-lived codebases
- AI-assisted refactoring where correctness matters
- Engineers who value precision over convenience

**Arbor is not for:**

- Small scripts or throwaway code
- Prompt-only workflows without code context
- People who just want autocomplete

## The Unified Nervous System

Arbor v1.0.0 represents the complete "Nervous System" for your code:

```
     Claude asks about AuthController
           │
           ▼
    ┌─────────────────┐
    │   Arbor Bridge  │  ← MCP Server (ArborQL)
    │   (arbor-mcp)   │     "find_path(Auth, DB)"
    └────────┬────────┘
             │ trigger_spotlight()
             ▼
    ┌─────────────────┐
    │   SyncServer    │  ← WebSocket broadcast
    │   (port 8080)   │
    └────────┬────────┘
             │ FocusNode message
     ┌───────┴───────┐
     │               │
     ▼               ▼
┌─────────┐    ┌─────────┐
│ VS Code │    │  Forest │
│ Golden  │    │ Camera  │
│Highlight│    │Animation│
│ #FFD700 │    │ 600ms   │
└─────────┘    └─────────┘
```

## CLI Commands

| Command | Description |
|---------|-------------|
| `arbor init` | Creates `.arbor/` config directory |
| `arbor index` | Full index of the codebase |
| `arbor query <q>` | Search the graph |
| `arbor serve` | Start the sidecar server |
| `arbor export` | Export graph to JSON |
| `arbor status` | Show index status |
| `arbor viz` | Launch the Logic Forest visualizer |
| `arbor bridge` | Start MCP server for AI integration |
| `arbor bridge --viz` | MCP + Visualizer together |
| `arbor check-health` | System diagnostics and health check |
| ⭐ `arbor refactor` | Safe refactor with blast radius preview (v1.1.0) |
| ⭐ `arbor explain` | Graph-backed code explanation (v1.1.0) |

## Troubleshooting

### Why does impact analysis return 0 nodes?

- The target node may not exist in the graph. Run `arbor query "NodeName"` to verify.
- The node may have no dependents (nothing calls or imports it).
- Try increasing depth: `arbor refactor NodeName --depth 5`.

### Why do Flutter widgets behave differently?

- Flutter uses composition, not inheritance. Arbor tracks `contains` edges, not class hierarchies.
- Use `arbor refactor WidgetName --depth 2` to see nested widgets.

### When to use --follow-symlinks?

- In monorepos with linked packages (pnpm, npm link).
- When local dependencies are symlinked into `node_modules`.
- Default is OFF to avoid infinite loops from circular links.

### Graph is empty after indexing?

- Check that your files use supported extensions: `.rs`, `.ts`, `.tsx`, `.py`, `.dart`, `.go`.
- Ensure files are not excluded by `.gitignore`.
- Run `arbor status` to see which extensions were detected.

## License

MIT — use it however you want. See [LICENSE](LICENSE) for details.

---

<p align="center">
  <strong>Built for developers who think code is more than text.</strong>
</p>

<p align="center">
  <em>"The forest is mapped. The AI is walking the path."</em>
</p>

<p align="center">
  <a href="https://github.com/Anandb71/arbor">⭐ Star us on GitHub</a>
</p>
