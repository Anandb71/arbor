# Arbor v1.6.0: Workflow Integration & Trust

**"Know what breaks, and prove it before merge."**

Arbor v1.6.0 focuses on production workflow reliability: safer PR gates, faster incremental indexing, and stronger release documentation.

## 🚀 Highlights

### 1. Git-Aware Change Intelligence

New commands for branch-aware analysis:

- `arbor diff` — preview blast radius for current git changes
- `arbor check` — CI safety gate with risk thresholding
- `arbor open <symbol>` — jump directly to the affected location

### 2. Incremental Developer Loop

- `arbor index --changed-only` re-indexes only changed files for faster feedback cycles.
- Improved changed-file heuristics for realistic branch workflows.

### 3. Better Snapshot Warm Starts

Arbor now persists and loads both:

- `.arbor/graph.bin`
- `.arbor/graph.json`

This improves startup and repeated-query performance across commands.

### 4. Diff Heuristic Reliability

Edge-case handling tightened for real-world repos:

- rename-aware changed-file detection
- whitespace-only diffs ignored
- generated/internal files filtered from impact input

### 5. Documentation & Release Professionalization

Repository docs were refreshed for March 2026 standards:

- branch strategy (`main`, `release/v1.5`, `release/v1.6`)
- CI/reproducibility guidance
- MCP directory visibility and integration clarity
- protocol and architecture docs alignment with current command surface

## ✅ Verification Summary

Validated against workflow-equivalent Rust CI steps:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features`
- `cargo build --all --verbose`
- `cargo test --all --verbose`

Integration tests include diff edge cases for rename/whitespace/generated-file scenarios.

## 🔧 Upgrade Notes

- Teams can adopt incremental refresh with:
  - `arbor index --changed-only`
- CI systems can enforce risk policy with:
  - `arbor check --json --max-blast-radius <N>`

## 📚 Related Docs

- [Quickstart](./QUICKSTART.md)
- [Installation Guide](./INSTALL.md)
- [MCP Integration](./MCP_INTEGRATION.md)
- [Protocol](./PROTOCOL.md)
- [Architecture](./ARCHITECTURE.md)
