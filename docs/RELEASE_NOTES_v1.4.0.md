# Arbor v1.4.0: The Trust Update 🌲

**"Know what breaks before you break it."**

This release transforms Arbor from a CLI utility into a complete **trust layer** for your codebase. It introduces a native GUI, Explainable Confidence scores, and a unified installation path.

## 🚀 Key Features

### 1. Native GUI (`arbor gui`)
A lightweight, fast, privacy-first interface for impact analysis. 
- **Focus**: Enter a symbol, see the blast radius.
- **Privacy**: File paths are hidden behind spoilers until you click them.
- **Speed**: Built on `egui` for instant startup and low memory usage.

### 2. Explainable Confidence
Arbor no longer just lists nodes—it tells you how much to trust the result.
- **🟢 High**: Static, well-connected, unambiguous.
- **🟡 Medium**: Some dynamic dispatch or indirection.
- **🔴 Low**: Heuristic matches or isolated nodes.
- **Node Roles**: See at a glance if a function is an `Entry Point`, `Utility`, `Core Logic`, or `Adapter`.

### 3. "Blessed" Installation
Zero confusion. One command to get everything:
```bash
cargo install arbor-graph-cli arbor-gui
```
This installs both the CLI (`arbor`) and the GUI binary, ensuring `arbor gui` works out of the box.

### 4. Reality-Proofing
Improved heuristics for:
- **Flutter/Dart**: Widget tree detection.
- **Dynamic Calls**: Better "Uncertain Edge" handling.
- **Monorepos**: Symlink support via `--follow-symlinks`.

## 🛠️ Fixes & Polish
- **CI/CD**: Fixed Linux build failures (missing GTK deps).
- **Window Icon**: Now correctly uses the Arbor logo (SVG rasterization).
- **Formatting**: Resolved `rustfmt` crashes on Windows.

---

**Full Changelog**: https://github.com/Anandb71/arbor/compare/v1.3.0...v1.4.0
