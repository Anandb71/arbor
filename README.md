<p align="center">
  <img src="docs/assets/arbor-logo.svg" alt="Arbor" width="120" height="120" />
</p>

# Arbor

**Graph‑Native Intelligence for Codebases**

> Know what breaks *before* you break it.

<p align="center">
  <a href="https://github.com/Anandb71/arbor/actions"><img src="https://img.shields.io/github/actions/workflow/status/Anandb71/arbor/rust.yml?style=flat-square&label=CI" alt="CI" /></a>
  <img src="https://img.shields.io/badge/release-1.6.1.1%20maintenance%20%7C%20main%20active-blue?style=flat-square" alt="Release channels" />
  <a href="https://glama.ai/mcp/servers/@Anandb71/arbor"><img src="https://img.shields.io/badge/MCP%20Directory-Glama-6f42c1?style=flat-square" alt="Glama MCP Directory" /></a>
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License" />
</p>

## Status (March 2026)

- **Stable maintenance line:** `release/v1.6` (`v1.6.1.1`)
- **Main development trunk:** `main`

Arbor `v1.6.1.1` is the active maintenance cut as of **March 18, 2026**.

## Highlights

- **Accurate Token Counting** — tiktoken (cl100k_base) replaces heuristic estimates for precise LLM context budgets
- **Fuzzy Symbol Suggestions** — Typo tolerance with Jaro-Winkler matching: `arbor refactor autth` → "Did you mean: `auth`?"
- **Enhanced MCP/AI Integration** — Rich JSON output with confidence, roles, and edge explanations for Claude/Cursor
- **Git-Aware Risk Workflows** — `arbor diff`, `arbor check`, and `arbor open` for refactor confidence
- **Incremental Refresh** — `arbor index --changed-only` for faster re-index during active branches
- **Better Python UX** — Empty `__init__.py` handled silently (no false warnings)

<p align="center">
  <img src="docs/assets/arbor-demo.gif" alt="Arbor refactor demo" width="700" />
</p>

## What is Arbor?

Arbor is a **local‑first impact analysis engine** for large codebases. Instead of treating code as text, Arbor parses your project into a **semantic dependency graph**. This lets you trace *real execution paths*—callers, callees, imports, inheritance, and cross‑file relationships—so you can confidently understand the consequences of change.

Unlike keyword search or vector‑based RAG systems, Arbor answers questions like:

> *“If I change this function, what actually breaks?”*

with **structural certainty**, not probabilistic guesses.

---

## Example: Blast Radius Detection

Before refactoring `detect_language`, inspect its true impact:

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

This is **execution‑aware analysis**, not text matching.

---

## Graphical Interface

Arbor ships with a **native GUI** for interactive impact analysis.

```bash
arbor gui
```

![Arbor GUI](docs/gui_screenshot.png)

### GUI Capabilities

* **Symbol Search** – Instantly locate functions, classes, and methods
* **Impact Visualization** – Explore direct and transitive dependencies
* **Privacy‑Safe** – File paths are hidden by default for clean screenshots
* **Export** – Copy results as Markdown for PRs and design docs

> The CLI and GUI share the *same* analysis engine—no feature gaps.

---

## Quick Start

1. **Install Arbor** (CLI + GUI):

   ```bash
   cargo install arbor-graph-cli
   ```

  Or use one-command installers (no Rust toolchain required):

  - macOS/Linux: `curl -fsSL https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.sh | bash`
  - Windows (PowerShell): `irm https://raw.githubusercontent.com/Anandb71/arbor/main/scripts/install.ps1 | iex`

  See [Installation Guide](docs/INSTALL.md) for version pinning and manual assets.

2. **One-shot setup + first index**:

  ```bash
  cd your-project
  arbor setup
  ```

3. **Run Impact Analysis**:

   ```bash
   arbor refactor <symbol-name>
   ```

  For git-aware workflows:

  ```bash
  arbor diff
  arbor check --max-blast-radius 30
  arbor open <symbol>
  ```

4. **Launch the GUI**:

   ```bash
   arbor gui
   ```

> You can run Arbor from any nested subdirectory; it automatically resolves to your project root.

📘 See the [Quickstart Guide](docs/QUICKSTART.md) for advanced workflows.

---

## Release Channels & Branches

To keep maintenance and feature work clean:

- `main` → ongoing development
- `release/v1.6` → maintenance-only fixes for 1.6.x (`v1.6.1.1` current)
- `release/v1.5` → legacy maintenance/backport branch

This avoids shipping new features into older maintenance branches and keeps backports explicit.

---

## MCP Directory Listing

Arbor is listed on Glama MCP Directory:

- **Glama:** https://glama.ai/mcp/servers/@Anandb71/arbor

<p align="center">
  <a href="https://glama.ai/mcp/servers/@Anandb71/arbor">
    <img width="380" height="200" src="https://glama.ai/mcp/servers/@Anandb71/arbor/badge" />
  </a>
</p>

---

## GitHub Marketplace Action

Arbor now includes a reusable GitHub Action at the repo root (`action.yml`) so teams can run impact analysis directly in CI.

### Quick usage

```yaml
name: Arbor Check
on: [pull_request]

jobs:
  arbor:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      - uses: Anandb71/arbor@release/v1.6
        with:
          command: check . --max-blast-radius 30
```

Inputs:

- `command` (default: `status .`) — Arbor subcommand + args (without leading `arbor`)
- `arbor-version` (default: `latest`) — install from crates.io at a pinned version
- `install-from-source` (default: `false`) — build from this repository source

> To publish this action in GitHub Marketplace: create a release tag (for example `v1`), add marketplace listing metadata in the release flow, and submit from the repository's Marketplace listing page.

---

## Documentation Hub

- **Quickstart:** [docs/QUICKSTART.md](docs/QUICKSTART.md)
- **Installation:** [docs/INSTALL.md](docs/INSTALL.md)
- **Architecture:** [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- **MCP Integration:** [docs/MCP_INTEGRATION.md](docs/MCP_INTEGRATION.md)
- **Protocol Specification:** [docs/PROTOCOL.md](docs/PROTOCOL.md)
- **Roadmap:** [docs/ROADMAP.md](docs/ROADMAP.md)
- **Release Notes (v1.6):** [docs/RELEASE_NOTES_v1.6.0.md](docs/RELEASE_NOTES_v1.6.0.md)
- **Release Notes (v1.6.1.1):** [docs/RELEASE_NOTES_v1.6.1.1.md](docs/RELEASE_NOTES_v1.6.1.1.md)

---

## Why Arbor?

Most AI coding tools treat code as **unstructured text**, relying on vector similarity. This approach is fast—but imprecise.

**Arbor builds a graph.**

Every function, class, and module is a node. Every call, import, and reference is an edge. When you ask a question, Arbor follows the graph—*the same way your program executes*.

```text
Traditional RAG:              Arbor Graph Analysis:

"auth" → 47 results          AuthController
(keyword similarity)           ├── calls → TokenMiddleware
                               ├── queries → UserRepository
                               └── emits → AuthEvent
```

The result: **deterministic, explainable answers**.

---

## Core Features

### Native GUI


A global symbol table resolves:

* Imports and re‑exports
* Inheritance and interfaces
* Overloads and namespaces

`User` in `auth.ts` is never confused with `User` in `types.ts`.

---

## Supported Languages

| Language       | Status | Parser Coverage                           |
| -------------- | ------ | ----------------------------------------- |
| **Rust**       | ✅      | Functions, Structs, Traits, Impls, Macros |
| **TypeScript** | ✅      | Classes, Interfaces, Types, Imports, JSX  |
| **JavaScript** | ✅      | Functions, Classes, Vars, Imports         |
| **Python**     | ✅      | Classes, Functions, Imports, Decorators   |
| **Go**         | ✅      | Structs, Interfaces, Funcs, Methods       |
| **Java**       | ✅      | Classes, Interfaces, Methods, Fields      |
| **C**          | ✅      | Structs, Functions, Enums, Typedefs       |
| **C++**        | ✅      | Classes, Namespaces, Templates            |
| **C#**         | ✅      | Classes, Methods, Properties, Interfaces  |
| **Dart**       | ✅      | Classes, Mixins, Widgets                  |

> **Python note:** Decorators, `__init__.py`, and `@dataclass` are statically analyzed. Dynamic dispatch is flagged with reduced confidence.

---

## Build from Source

```bash
git clone https://github.com/Anandb71/arbor.git
cd arbor
cargo build --release
cargo test
```

### Frictionless setup notes

- **No external graph database is required.** Arbor is local-first and uses embedded storage.
- From a fresh clone, contributors can start with just:
  - `cargo build`
  - `cargo test`

If you prefer containerized setup, run:

```bash
docker compose up --build arbor-dev
```

### Linux GUI Dependencies

```bash
sudo apt-get install -y pkg-config libx11-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libgtk-3-dev libfontconfig1-dev libasound2-dev libssl-dev cmake
```

---

## Troubleshooting

### Symbol not found?

* **.gitignore** – Arbor respects it (`arbor status --files`)
* **File type** – Ensure the extension is supported
* **Empty files** – Skipped (except `__init__.py`)
* **Dynamic calls** – `eval` / runtime reflection may not resolve
* **Case sensitivity** – Use `arbor query <partial>` to search

### Empty graph?

Run `arbor status` to verify file detection and parser health.

### Need environment diagnostics?

Run `arbor doctor` (or `arbor check-health`) to verify ports, project structure, and integration readiness.

### Repo suddenly huge (multi-GB)?

Rust and Flutter build artifacts can grow quickly during iterative testing.

- Windows PowerShell: `./scripts/clean.ps1`
- macOS/Linux: `./scripts/clean.sh`
- Deeper cleanup (also removes local Arbor/Flutter cache artifacts):
  - PowerShell: `./scripts/clean.ps1 -Deep`
  - Bash: `./scripts/clean.sh --deep`

This is safe for source code; it only removes generated artifacts that can be rebuilt.

---

## Security Model

Arbor is **Local‑First by design**:

* No data exfiltration
* Fully offline
* No API keys
* Fully open source

Your code never leaves your machine.

---

## License

MIT License. See [LICENSE](LICENSE) for details.

<p align="center">
  <a href="https://github.com/Anandb71/arbor">⭐ Star Arbor on GitHub</a>
</p>

---

## Contributors

<!-- CONTRIBUTORS:START -->
<p align="center">
    <a href="https://github.com/Anandb71" title="Anandb71" style="text-decoration:none; margin:6px; display:inline-block;">
        <img src="https://avatars.githubusercontent.com/u/169837340?v=4" alt="Anandb71" width="72" height="72" loading="lazy" style="border-radius:50%; border:2px solid #30363d; box-sizing:border-box;" />
  </a>
    <a href="https://github.com/holg" title="holg" style="text-decoration:none; margin:6px; display:inline-block;">
        <img src="https://avatars.githubusercontent.com/u/1383439?v=4" alt="holg" width="72" height="72" loading="lazy" style="border-radius:50%; border:2px solid #30363d; box-sizing:border-box;" />
  </a>
    <a href="https://github.com/cabinlab" title="cabinlab" style="text-decoration:none; margin:6px; display:inline-block;">
        <img src="https://avatars.githubusercontent.com/u/66889299?v=4" alt="cabinlab" width="72" height="72" loading="lazy" style="border-radius:50%; border:2px solid #30363d; box-sizing:border-box;" />
  </a>
    <a href="https://github.com/Karthiksenthilkumar1" title="Karthiksenthilkumar1" style="text-decoration:none; margin:6px; display:inline-block;">
        <img src="https://avatars.githubusercontent.com/u/182195883?v=4" alt="Karthiksenthilkumar1" width="72" height="72" loading="lazy" style="border-radius:50%; border:2px solid #30363d; box-sizing:border-box;" />
  </a>
    <a href="https://github.com/sanjayy-j" title="sanjayy-j" style="text-decoration:none; margin:6px; display:inline-block;">
        <img src="https://avatars.githubusercontent.com/u/178475117?v=4" alt="sanjayy-j" width="72" height="72" loading="lazy" style="border-radius:50%; border:2px solid #30363d; box-sizing:border-box;" />
  </a>
    <a href="https://github.com/sathguru07" title="sathguru07" style="text-decoration:none; margin:6px; display:inline-block;">
        <img src="https://avatars.githubusercontent.com/u/182798669?v=4" alt="sathguru07" width="72" height="72" loading="lazy" style="border-radius:50%; border:2px solid #30363d; box-sizing:border-box;" />
  </a>
</p>
<p align="center"><sub><strong>6 contributors</strong> | <a href="https://github.com/Anandb71/arbor/graphs/contributors">View all</a></sub></p>

<!-- CONTRIBUTORS:END -->
