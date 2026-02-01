<p align="center">
  <img src="docs/assets/arbor-logo.svg" alt="Arbor" width="120" height="120" />
</p>

# Arbor v1.5.0

**Graph‑Native Intelligence for Codebases**

> Know what breaks *before* you break it.

<p align="center">
  <a href="https://github.com/Anandb71/arbor/actions"><img src="https://img.shields.io/github/actions/workflow/status/Anandb71/arbor/rust.yml?style=flat-square&label=CI" alt="CI" /></a>
  <img src="https://img.shields.io/badge/release-v1.5.0-blue?style=flat-square" alt="Release" />
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License" />
</p>

## What's New in v1.5

- **Accurate Token Counting** — tiktoken (cl100k_base) replaces heuristic estimates for precise LLM context budgets
- **Fuzzy Symbol Suggestions** — Typo tolerance with Jaro-Winkler matching: `arbor refactor autth` → "Did you mean: `auth`?"
- **Enhanced MCP/AI Integration** — Rich JSON output with confidence, roles, and edge explanations for Claude/Cursor
- **GUI Version Watermark** — "Arbor v1.5" badge for brand visibility in screenshots
- **Better Python UX** — Empty `__init__.py` handled silently (no false warnings)

<p align="center">
  <img src="docs/assets/arbor-demo.gif" alt="Arbor refactor demo" width="700" />
</p>

## Overview
---

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

Arbor v1.4 ships with a **native GUI** for interactive impact analysis.

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

2. **Run Impact Analysis**:

   ```bash
   cd your-project
   arbor refactor <symbol-name>
   ```

3. **Launch the GUI**:

   ```bash
   arbor gui
   ```

📘 See the [Quickstart Guide](docs/QUICKSTART.md) for advanced workflows.

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

A lightweight, high‑performance interface bundled directly with Arbor—no browser, no server.

### Confidence Scoring

Each result includes an **explainable confidence level**:

* **High** – Fully resolved, statically verifiable paths
* **Medium** – Partial uncertainty (e.g., polymorphism)
* **Low** – Heuristic or dynamic resolution

### Node Classification

Arbor infers architectural roles automatically:

* **Entry Point** – APIs, CLIs, main functions
* **Core Logic** – Domain and business rules
* **Utility** – Widely reused helpers
* **Adapter** – Interfaces, boundaries, and bridges

### AI Bridge (MCP)

Arbor implements the **Model Context Protocol (MCP)**, enabling LLMs (e.g., Claude) to query the graph directly:

* `find_path(start, end)` – Trace logic flow
* `analyze_impact(node)` – Compute blast radius
* `get_context(node)` – Retrieve semantically related code

### Cross‑File Resolution

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
cd arbor/crates
cargo build --release
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
