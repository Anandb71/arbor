# Arbor Roadmap: Path to v2.0 & Beyond

> **Vision:** Arbor is the "Nervous System" for AI Agents—a persistent, visual, and intelligent memory graph that prevents hallucinations and enables safe, massive-scale refactoring.

---

## ✅ Current Execution Status (2026-03-25)

This section tracks what is already shipped versus what remains strategic work.

### Shipped / In-Progress Foundation

- [x] **Agent Bridge (MCP):** Operational bridge and tool surface (`get_logic_path`, `analyze_impact`, `find_path`)
- [x] **`arbor audit` command foundation:** Security-path tracing is available in CLI
- [x] **Language expansion baseline:** JS/TS, Python, Go, Rust, Java, C/C++, C#, Dart parsing paths are present
- [x] **Language expansion wave (v1.6.2):** Kotlin, Swift, Ruby, PHP, and Shell are now indexed via fallback parser support
- [x] **Air-gapped local-first model:** Core workflows operate offline
- [x] **Graph snapshot persistence:** CLI now writes/loads `.arbor/graph.json` and `.arbor/graph.bin` for faster reuse across commands
- [x] **Git-aware impact workflows:** `arbor diff`, `arbor check`, `arbor open`, and `arbor index --changed-only`
- [x] **Diff edge-case test coverage:** rename, whitespace-only, and generated-file heuristics covered with integration tests
- [x] **Automated release pipeline (v1.7.0):** Cross-platform binary builds, crates.io, GHCR, VS Code Marketplace, Open VSX
- [x] **Multi-channel distribution (v1.7.0):** Homebrew, Scoop, npm, Docker
- [x] **VS Code extension v1.7.0:** 8 commands, quick-pick menu, walkthrough onboarding

### Still Outstanding (Major Epics)

- [ ] Persistent graph database (SQLite/Sled) with transactional updates and indexed queries
- [ ] Time-travel/git-history architectural drift analysis
- [ ] Compliance report generation (SOC2/ISO artifacts)
- [ ] Plugin system for community parser/runtime extensions
- [ ] Enterprise RBAC and deployment policy controls
- [ ] Learning loop from developer feedback and correction signals

---

## 🧠 1. Architectural Memory Graph (Visual Intelligence)
*Turn impact analysis into a persistent, explorable map.*
- [ ] **Persistent Graph Store:** Move beyond ephemeral indexing to a persistent database (SQLite/Sled) for instant load times.
- [ ] **Visual Dependency Explorer:** Interactive, queryable UI to answer "What breaks if I delete this?"
- [ ] **Time-Travel Analysis:** Track architectural drift over time (integration with Git history).

## 🤖 2. AI Explanation Layer
*Make the graph human-readable and trustable.*
- [ ] **Narrative Engine:** Convert raw graph data into sentences (e.g., "This function affects 6 downstream services...").
- [ ] **Confidence Contracts:** SLA for analysis certainty (e.g., "100% static certainty" vs "80% heuristic").
- [ ] **Agent Bridge (MCP):** Deepen integration with Claude/Cursor to act as the "ground truth" for AI coding agents.

## 🛡️ 3. Security & Audit ("Blast Radius for CVEs")
*Penetrate the security market with vulnerability tracing.*
- [ ] **`arbor audit <function>`:** Trace tainted inputs and vulnerable execution paths.
- [ ] **Compliance Reports:** Generate artifacts for SOC2/ISO 27001 showing impact analysis.

## 🌍 4. Multi-Language & Ecosystem
*Be the #1 tool for every stack.*
- [x] **Language Expansion:** Full support for JS/TS, Python, Go, Rust, Java, C#.
- [x] **Language Expansion (v1.6.2):** Added pragmatic parser support for Kotlin, Swift, Ruby, PHP, and Shell.
- [ ] **Plugin System:** Wasm-based plugin architecture for community parsers.
- [ ] **"Bounty Board":** Gamified community contributions for new language parsers.

## 🏢 5. Enterprise Mode
*Features for global dominance.*
- [ ] **Air-Gapped Support:** Fully offline operation (already core, but explicit support).
- [ ] **On-Premise Deployment:** Dockerized containers for enterprise CI/CD.
- [ ] **Role-Based Access:** Graph views tailored for Junior vs Senior devs vs Architects.

## 🔄 6. Continuous Learning Engine
*From rule-based to intelligent.*
- [ ] **Feedback Loop:** Learn from user corrections ("No, this isn't a dependency") to improve heuristics.
- [ ] **Pattern Recognition:** Automatically detect and adapt to repo-specific architectural patterns (e.g., "All `*Service` classes are singletons").

---

## 🚀 Immediate Focus (v1.8 / v2.0)
**Theme:** *Persistent Intelligence & Enterprise Readiness*

1. **Persistent Graph Database:** Move to SQLite/Sled with transactional updates and indexed queries for instant load.
2. **Plugin System (Wasm):** Community-extensible parser and runtime plugins.
3. **Time-Travel Analysis:** Git-history-aware architectural drift detection.
4. **Security Layer Expansion:** Deepen `arbor audit` with taint-style path confidence and compliance report generation.
