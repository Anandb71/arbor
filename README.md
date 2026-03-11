<p align="center">
  <img src="docs/assets/arbor-logo.svg" alt="Arbor" width="120" height="120" />
</p>

# Arbor

**Graph‑Native Intelligence for Codebases**

> Know what breaks *before* you break it.

<p align="center">
  <a href="https://github.com/Anandb71/arbor/actions"><img src="https://img.shields.io/github/actions/workflow/status/Anandb71/arbor/rust.yml?style=flat-square&label=CI" alt="CI" /></a>
  <img src="https://img.shields.io/badge/release-1.5%20stable%20%7C%201.6%20in%20progress-blue?style=flat-square" alt="Release channels" />
  <a href="https://glama.ai/mcp/servers/@Anandb71/arbor"><img src="https://img.shields.io/badge/MCP%20Directory-Glama-6f42c1?style=flat-square" alt="Glama MCP Directory" /></a>
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License" />
</p>

## Status (March 2026)

- **Stable line:** `release/v1.5`
- **Current feature line:** `release/v1.6`
- **Main development trunk:** `main`

Arbor is currently in final polish for the v1.6 release line.

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
- `release/v1.5` → maintenance-only fixes for 1.5.x
- `release/v1.6` → 1.6 feature delivery and stabilization

This avoids shipping new features into older maintenance branches and keeps backports explicit.

---

## MCP Directory Listing

Arbor is listed on Glama MCP Directory:

- **Glama:** https://glama.ai/mcp/servers/@Anandb71/arbor
## Contributors

<p align="center">
  <a href="https://glama.ai/mcp/servers/@Anandb71/arbor">
    <img width="380" height="200" src="https://glama.ai/mcp/servers/@Anandb71/arbor/badge" />
  <a href="https://github.com/Anandb71" title="Anandb71">
    <img src="https://avatars.githubusercontent.com/u/169837340?v=4" width="64" height="64" alt="Anandb71" />
  </a>
  <a href="https://github.com/holg" title="holg">
    <img src="https://avatars.githubusercontent.com/u/1383439?v=4" width="64" height="64" alt="holg" />
  </a>
  <a href="https://github.com/apps/copilot-swe-agent" title="Copilot">
    <img src="https://avatars.githubusercontent.com/in/1143301?v=4" width="64" height="64" alt="Copilot" />
  </a>
  <a href="https://github.com/cabinlab" title="cabinlab">
    <img src="https://avatars.githubusercontent.com/u/66889299?v=4" width="64" height="64" alt="cabinlab" />
  </a>
  <a href="https://github.com/Karthiksenthilkumar1" title="Karthiksenthilkumar1">
    <img src="https://avatars.githubusercontent.com/u/182195883?v=4" width="64" height="64" alt="Karthiksenthilkumar1" />
  </a>
  <a href="https://github.com/sanjayy-j" title="sanjayy-j">
    <img src="https://avatars.githubusercontent.com/u/178475117?v=4" width="64" height="64" alt="sanjayy-j" />
  </a>
  <a href="https://github.com/sathguru07" title="sathguru07">
    <img src="https://avatars.githubusercontent.com/u/182798669?v=4" width="64" height="64" alt="sathguru07" />
  </a>
</p>
