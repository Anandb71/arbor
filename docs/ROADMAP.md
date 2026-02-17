# Arbor Roadmap: Path to v2.0 & Beyond

> **Vision:** Arbor is the "Nervous System" for AI Agents—a persistent, visual, and intelligent memory graph that prevents hallucinations and enables safe, massive-scale refactoring.

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
- [ ] **Language Expansion:** Full support for JS/TS, Python, Go, Rust, Java, C#.
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

## 🚀 Immediate Focus (v1.6)
**Theme:** *The Security & Intelligence Layer*

1. **`arbor audit` Command:** Trace impact of specific symbols with a security focus.
2. **Docs & Community:** `CONTRIBUTING.md`, Bounty Board, and "Why Impact Analysis Fails" blog context.
3. **Visualizer Polish:** Advanced filtering and "professional" UI overhaul.
