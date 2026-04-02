<p align="center">
  <img src="docs/assets/arbor-logo.svg" alt="Arbor logo" width="120" height="120" />
</p>

# Arbor

**Graph-native intelligence for codebases.**

Know what breaks *before* you break it.

<p align="center">
  <a href="https://github.com/Anandb71/arbor/actions"><img src="https://img.shields.io/github/actions/workflow/status/Anandb71/arbor/rust.yml?style=flat-square&label=Rust%20CI" alt="Rust CI" /></a>
  <a href="https://crates.io/crates/arbor-graph-cli"><img src="https://img.shields.io/crates/v/arbor-graph-cli?style=flat-square&label=crates.io" alt="Crates.io" /></a>
  <a href="https://github.com/Anandb71/arbor/releases"><img src="https://img.shields.io/github/v/release/Anandb71/arbor?style=flat-square&label=release" alt="Latest release" /></a>
  <a href="https://github.com/Anandb71/arbor/pkgs/container/arbor"><img src="https://img.shields.io/badge/GHCR-container-blue?style=flat-square" alt="GHCR" /></a>
  <a href="https://glama.ai/mcp/servers/@Anandb71/arbor"><img src="https://img.shields.io/badge/MCP%20Directory-Glama-6f42c1?style=flat-square" alt="Glama MCP Directory" /></a>
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="MIT License" />
</p>

---

## Table of Contents

- [Why Arbor](#why-arbor)
- [What you get](#what-you-get)
- [Visual tour](#visual-tour)
- [Quickstart](#quickstart)
- [Installation options](#installation-options)
- [MCP integration](#mcp-integration)
- [Language support](#language-support)
- [Architecture and docs](#architecture-and-docs)
- [Git-aware CI workflows](#git-aware-ci-workflows)
- [Release channels](#release-channels)
- [Contributing](#contributing)
- [Security](#security)
- [License](#license)

---

## Why Arbor

Most AI code tooling treats code as text retrieval.

Arbor builds a **semantic dependency graph** and answers execution-aware questions:

- *If I change this symbol, what breaks?*
- *Who calls this function, directly and transitively?*
- *What is the shortest architectural path between these two nodes?*

You get deterministic, explainable impact analysis instead of approximate keyword matches.

---

## What you get

- **Blast radius analysis** with confidence levels and role classification
- **Graph-backed symbol resolution** across files and language boundaries
- **CLI + GUI + MCP bridge** sharing the same analysis engine
- **Incremental indexing** for fast inner-loop development
- **Git-aware checks** for pull-request risk gates

---

## Visual tour

<p align="center">
  <img src="docs/assets/arbor-demo.gif" alt="Arbor demo animation" width="760" />
</p>

<p align="center">
  <img src="docs/assets/visualizer-screenshot.png" alt="Arbor visualizer screenshot" width="760" />
</p>

For a full-screen recording of the workflow, see [media/recording-2026-01-13.mp4](media/recording-2026-01-13.mp4).

---

## Quickstart

```bash
# 1) Install Arbor CLI
cargo install arbor-graph-cli

# 2) Initialize in your repository
cd your-project
arbor setup

# 3) Explore impact before refactor
arbor refactor <symbol-name>

# 4) Optional: run git-aware checks
arbor diff
arbor check --max-blast-radius 30

# 5) Launch GUI
arbor gui
```

---

## Installation options

Use whichever channel fits your environment:

```bash
# Rust / Cargo
cargo install arbor-graph-cli

# Homebrew (macOS/Linux)
brew install Anandb71/tap/arbor

# Scoop (Windows)
scoop bucket add arbor https://github.com/Anandb71/arbor
scoop install arbor

# npm wrapper (cross-platform)
npx @anandb71/arbor-cli

# Docker
docker pull ghcr.io/anandb71/arbor:latest
```

No-Rust installers:

- macOS/Linux: `curl -fsSL https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.sh | bash`
- Windows PowerShell: `irm https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.ps1 | iex`

For pinned/versioned installs, see [docs/INSTALL.md](docs/INSTALL.md).

---

## MCP integration

Arbor includes a real MCP server via `arbor bridge` (stdio transport).

### Claude Code quick install

```bash
claude mcp add --transport stdio --scope project arbor -- arbor bridge
claude mcp list
```

### Multi-client setup

- Full guide: [docs/MCP_INTEGRATION.md](docs/MCP_INTEGRATION.md)
- Ready templates: [`templates/mcp/`](templates/mcp/)
- Bootstrap scripts:
  - `scripts/setup-mcp.sh`
  - `scripts/setup-mcp.ps1`

### Registry verification (authoritative)

- Registry name: `io.github.Anandb71/arbor`
- Official API lookup: https://registry.modelcontextprotocol.io/v0.1/servers?search=io.github.Anandb71/arbor

> [!NOTE]
> `github.com/mcp` search UI may lag indexing. Use the official registry API lookup above as source of truth.

---

## Language support

Arbor supports production parsing and graph analysis across major ecosystems:

- Rust
- TypeScript / JavaScript
- Python
- Go
- Java
- C / C++
- C#
- Dart
- Kotlin (fallback parser)
- Swift (fallback parser)
- Ruby (fallback parser)
- PHP (fallback parser)
- Shell (fallback parser)

Detailed parser notes and expansion guidance:

- [docs/ADDING_LANGUAGES.md](docs/ADDING_LANGUAGES.md)
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)

---

## Architecture and docs

Start here when you need deeper internals:

- [docs/QUICKSTART.md](docs/QUICKSTART.md)
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- [docs/GRAPH_SCHEMA.md](docs/GRAPH_SCHEMA.md)
- [docs/PROTOCOL.md](docs/PROTOCOL.md)
- [docs/MCP_INTEGRATION.md](docs/MCP_INTEGRATION.md)
- [docs/ROADMAP.md](docs/ROADMAP.md)

---

## Git-aware CI workflows

Arbor supports pre-merge risk checks and change gating:

```bash
arbor diff
arbor check --max-blast-radius 30
arbor open <symbol>
```

Use the repository GitHub Action for CI integration:

```yaml
name: Arbor Check
on: [pull_request]

jobs:
  arbor:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      - uses: Anandb71/arbor@release/v1.8
        with:
          command: check . --max-blast-radius 30
```

---

## Release channels

Automated release distribution includes:

- GitHub Releases (platform binaries)
- crates.io
- GHCR container images
- npm wrapper package
- VS Code Marketplace / Open VSX extension channels
- Homebrew + Scoop

Runbook: [docs/RELEASING.md](docs/RELEASING.md)

---

## Contributing

Contributions are welcome.

- Start with: [CONTRIBUTING.md](CONTRIBUTING.md)
- Code of conduct: [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- Security policy: [SECURITY.md](SECURITY.md)
- Good first tasks: [docs/GOOD_FIRST_ISSUES.md](docs/GOOD_FIRST_ISSUES.md)

For local development:

```bash
cargo build --workspace
cargo test --workspace
```

---

## Security

Arbor is local-first by design:

- No mandatory data exfiltration
- Offline-capable workflows
- Open-source code paths

Report vulnerabilities via [SECURITY.md](SECURITY.md).

---

## License

MIT — see [LICENSE](LICENSE).
