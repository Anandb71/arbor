# Arbor Roadmap

> **Mission:** Replace embedding-based RAG for code with deterministic, graph-based context.

## North Star Metric

**v1.1.0 Success Criteria:**

- Context size reduced by 60%+ vs naive file RAG
- Zero unrelated files injected into LLM context
- High-precision detection of graph-reachable downstream breaks

---

## Correctness Contract

Arbor guarantees:

- ✅ All returned context is **graph-reachable** from the queried node
- ✅ **No unrelated files** are injected into prompts
- ✅ Every dependency is **traceable and inspectable**
- ✅ All commands support `--why` for auditability

---

## Non-Goals

Arbor does **not** aim to:

- ❌ Generate code (that's the LLM's job)
- ❌ Replace your IDE
- ❌ Guess runtime behavior or dynamic dispatch
- ❌ Provide fuzzy "maybe related" results

Arbor exists to provide **correct structure**, not creativity.

---

## v1.1.0 "The Sentinel Update" (Must-Ship)

**Theme:** *"Predict breakage + give AI only the logic it needs"*

### 🎯 1. Impact Radius Simulator

Predict which nodes break before you change code.

```bash
arbor refactor auth.rs::validate_token --why
# ⚠️  Blast Radius: 12 nodes affected
# ├── Direct: TokenMiddleware (calls validate_token)
# ├── Transitive: UserService, SessionManager
# └── Public API impact: 2 endpoints
```

| Component | File |
|-----------|------|
| BFS/DFS Reachability | `arbor-graph/src/impact.rs` |
| MCP Tool | `arbor-mcp/src/lib.rs` |
| Heat Gradient (hop distance) | `visualizer/lib/graph/` |

---

### ✂️ 2. Dynamic Context Slicing

Send only relevant code to LLMs, not entire files.

```bash
arbor context api::handler --max-tokens 4000 --why
# Included: handler.rs (entry), auth.rs (calls), db.rs (queries)
# Excluded: tests/, docs/, unrelated modules
# Total: 3,200 tokens (within limit)
```

**Refinements:**

- **Pinning**: Mark core nodes (e.g., `auth`, `config`) to always include
- **Token estimator**: Approximate token count before sending

---

### 🚀 3. Opinionated Workflows

Two killer CLI commands that make Arbor undeniable:

#### `arbor refactor <node>`

Safe refactor mode with blast radius preview.

#### `arbor explain "<question>"`

Graph-backed codebase explanation.

```bash
arbor explain "Where does authentication start?"
# Path: AuthController → TokenMiddleware → UserRepository → DB
# Context: 2,400 tokens | Confidence: Graph-backed (not RAG)
```

---

### � 4. AI Transcript Demos

Before/after comparisons showing LLM behavior with vs without Arbor.

- `docs/demos/WITHOUT_ARBOR.md` — hallucinations, missed deps
- `docs/demos/WITH_ARBOR.md` — correct paths, minimal context

---

### 🐳 5. Docker + Cross-Platform CI

One-command installation on any platform.

```bash
docker pull ghcr.io/anandb71/arbor:1.1.0
docker run -v $(pwd):/workspace ghcr.io/anandb71/arbor index
```

| Target | Artifact |
|--------|----------|
| macOS Intel | `arbor-macos-intel.zip` |
| macOS ARM | `arbor-macos-arm64.zip` |
| Linux x64 | `arbor-linux-x64.tar.gz` |
| Linux ARM64 | `arbor-linux-arm64.tar.gz` |
| Windows | `arbor-windows.zip` |

---

## v1.2.0 "The Insight Update" (Deferred)

**Theme:** *"Understand code health and history"*

| Feature | Description |
|---------|-------------|
| 🔀 Shadow Indexing | Structural git diffs (`--structural-only`) |
| � Technical Debt Heatmaps | Cyclomatic complexity, coupling, cohesion |
| � Weekly Health Reports | Complexity trends, dead code detection |
| 🎮 Archipelago Mode | Filter noise, reveal architecture clusters |
| 🎯 TypeScript Depth | Flagship language: async edges, React trees |

---

## v1.3.0+ "The Ecosystem Update" (Future)

**Theme:** *"Collaboration and IDE integration"*

| Feature | Description |
|---------|-------------|
| 👥 Arbor Relay | Real-time collaborative graph sessions |
| 🔌 Full LSP Integration | Hover, CodeLens, Go to Definition |
| 🧪 What-If Sandbox | Simulate refactors without touching files |
| 📖 ArborQL Documentation | Full query syntax reference |
| 👋 Contributor Onboarding | Tutorials, ADRs, good first issues |

---

## Implementation Order

| Phase | Deliverables | Est. Effort |
|-------|--------------|-------------|
| **v1.1.0** | Impact Radius, Context Slicing, Workflows, Docker | 4 weeks |
| **v1.2.0** | Structural Diffs, Heatmaps, TS Depth | 4 weeks |
| **v1.3.0** | Relay, LSP, Sandbox | 6 weeks |

---

## v1.1.0 Checklist

- [ ] Implement `impact.rs` with BFS/DFS reachability
- [ ] Implement `slice.rs` with token estimation
- [ ] Add `arbor refactor` command
- [ ] Add `arbor explain` command
- [ ] Add `--why` flag to all commands
- [ ] Create AI transcript demos
- [ ] Set up Docker + GitHub Actions CI
- [ ] Write v1.1.0 release announcement

---

## Explicitly Out of Scope for v1.1.0

- ❌ Arbor Relay (collaboration)
- ❌ Full LSP integration
- ❌ Technical Debt Heatmaps
- ❌ Multi-language depth parity
- ❌ Enterprise features (RBAC, SSO)

These are valuable but not core to v1.1.0's identity.
