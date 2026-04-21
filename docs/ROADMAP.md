# Arbor Roadmap: Path to v2.0 & Beyond

> **Vision:** Arbor is the "Nervous System" for AI Agents—a persistent, visual, and intelligent memory graph that prevents hallucinations and enables safe, massive-scale refactoring.

---

## ✅ Current Execution Status (2026-03-25)

This section tracks what is already shipped versus what remains strategic work.

### Shipped / In-Progress Foundation (as of 2026-04-20)

- [x] **Agent Bridge (MCP):** Operational with expanded tools (`get_knowledge_path` with logic paths/Markdown links + explanations, `analyze_impact` with markdown table format for PR bot, `find_path`, sorted_by_centrality + ConfidenceExplanation)
- [x] **PR Bot polish:** Action + workflow + MCP markdown tables with **bold high-risk** using ConfidenceExplanation (professional audit reports)
- [x] **Tauri Pivot/Lattice:** Desktop shell with system tray (Personal OS feel), integrated graph/MCP (companion starter startup in Arbor v2/)
- [x] **`arbor audit` + impact workflows:** Git-aware, with blast radius for code PRs
- [x] **Language expansion:** Full with fallback_parser for Markdown (NodeKind::Section), Dart fixes, all tests passing
- [x] **Persistent store:** Sled in GraphStore with incremental updates, centrality precompute (Priority 2)
- [x] **Local-first air-gapped:** All core + Lattice MVP offline
- [x] **VS Code extension, releases, distribution:** Updated with PR bot foundation
- [x] **Eat own dog food:** Parser_v2 registry, Markdown support for Lattice/visualizer

### Still Outstanding (Major Epics)

- [ ] Full write-mode visualizer + 4D time-travel (Priority for 4 features)
- [ ] Plugin/WASM system for parsers
- [ ] Enterprise RBAC, compliance reports, learning loop
- [ ] Monetization for Lattice (sponsors, pro sync) + full GTM launch
- [ ] Deeper monorepo integration for Tauri + arbor crates

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
