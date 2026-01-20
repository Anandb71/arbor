# Arbor Roadmap: v1.3 → v2.0

> **Goal:** Arbor becomes the default pre-refactor safety tool for any developer, with a simple, intuitive GUI and zero guesswork.

---

## Phase 1: Hero Command Perfection (v1.3.x)

**Arbor must feel emotionally safe before adding new features.**

### ✅ Completed
- [x] Smart edge resolution
- [x] Persistent caching
- [x] Warm refactor output
- [x] Fallback suggestions
- [x] Quickstart guide

### 🔜 To Finish
- [ ] Tighten refactor output wording
- [ ] Improve fallback ranking
- [ ] More examples in README
- [ ] Add `arbor status --files` listing

**Outcome:** `arbor refactor <target>` becomes reliable, predictable, and friendly.

---

## Phase 2: GUI v1 — Minimal, Impact-First (v1.4)

**The GUI should ONLY exist to make the "What breaks if I change this?" moment obvious.**

### 🎯 Scope
- [ ] Add `arbor gui` mode
- [ ] Egui-based window (Rust native)
- [ ] Text box: "Enter symbol"
- [ ] Button: Analyze Impact
- [ ] Clean results panel:
  - Direct callers
  - Indirect callers
  - Downstream dependencies
- [ ] Copy-as-markdown button
- [ ] Light/Dark theme (egui built-in)

### ❌ Not in v1 GUI
- No graph
- No sidebar
- No settings
- No file explorer

**Outcome:** A single-window safety console any dev can understand in 10 seconds.

---

## Phase 3: Developer Trust Features (v1.5)

**Address the biggest real-world problem: "Can I trust this output?"**

- [ ] **Confidence Signal**: Low / Medium / High
- [ ] Explain WHY confidence is low (e.g., dynamic calls, missing edges)
- [ ] **Node Roles**:
  - Entry point
  - Utility
  - Core logic
  - Isolated
- [ ] Highlight missing edges as "Uncertain Areas"

**Outcome:** Arbor becomes transparent, not mysterious.

---

## Phase 4: Code Reality Support (v1.6)

**Real codebases aren't clean. Arbor must handle messiness.**

- [ ] Dynamic call heuristics
- [ ] Widget-tree heuristics for Flutter/Dart
- [ ] "Possible runtime edge" hints
- [ ] Smarter import resolution for JS/TS/Python
- [ ] Support for larger monorepos without noise

**Outcome:** Arbor works on ugly, real-world code — not just pretty examples.

---

## Phase 5: GUI v2 — Visual + Structured (v1.7)

**Now that trust is solid, add carefully scoped visual features.**

- [ ] Optional graph panel (not default view)
- [ ] Collapsible call tree
- [ ] File path → click to open in editor
- [ ] "Suggested safe refactors" section
- [ ] Search history list

**Outcome:** GUI becomes a real productivity tool, not a gimmick.

---

## Phase 6: Workflow Integration (v1.8–v1.9)

**Fit Arbor into developers' daily routines.**

- [ ] PR summary generator (markdown)
- [ ] AI-friendly JSON output modes
- [ ] Editor integrations:
  - Cursor
  - VS Code (simple extension)
- [ ] `arbor watch` mode: auto-refresh index on file save
- [ ] Configurable ignore patterns

**Outcome:** Arbor becomes something people use 5× per day, not once per week.

---

## Phase 7: v2.0 Identity Lock-In

**Promise:** *"If Arbor says a change is safe, you understand why."*

### Requirements
- [ ] GUI mature
- [ ] CLI output consistent and human-friendly
- [ ] Clear confidence/uncertainty signals
- [ ] Supports common real-world patterns (frameworks, widgets, async)
- [ ] Caching stable for large repos
- [ ] No empty or useless outputs
- [ ] Zero confusion around installation or crate name

**Outcome:** Arbor reaches **trusted tool status**.  
Not a toy. Not an experiment. Something developers depend on.

---

## Phase X: Optional Long-Term (Post-2.0)

*Only after adoption is strong.*

- [ ] Full-blown logic visualizer (rebuilt properly)
- [ ] Architecture smell detection
- [ ] Automated refactor suggestions
- [ ] LSP server
- [ ] Multi-project tagging (concepts from issue #32)

---

> **North Star:** Arbor is the tool you run *before* refactoring, not after something breaks.
