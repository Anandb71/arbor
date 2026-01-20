# Arbor Roadmap: v1.3 → v2.0

> **Goal:** Arbor becomes the default pre-refactor safety tool for any developer, with a simple, intuitive GUI and zero guesswork.

---

## Phase 1: Hero Command Perfection (v1.3.x) ✅

**Arbor must feel emotionally safe before adding new features.**

- [x] Smart edge resolution
- [x] Persistent caching
- [x] Warm refactor output
- [x] Fallback suggestions (with relevance ranking)
- [x] Quickstart guide
- [x] `arbor status --files` listing

**Outcome:** `arbor refactor <target>` is reliable, predictable, and friendly.

---

## Phase 2: GUI v1 — Minimal, Impact-First (v1.4) ✅

**The GUI should ONLY exist to make the "What breaks if I change this?" moment obvious.**

- [x] Add `arbor gui` mode
- [x] Egui-based window (Rust native)
- [x] Text box: "Enter symbol"
- [x] Button: Analyze Impact
- [x] Clean results panel (direct callers, indirect callers, dependencies)
- [x] Copy-as-markdown button
- [x] Light/Dark theme (egui built-in)

**Outcome:** A single-window safety console any dev can understand in 10 seconds.

---

## Phase 3: Developer Trust Features (v1.4) ✅

**Address the biggest real-world problem: "Can I trust this output?"**

- [x] **Confidence Signal**: Low / Medium / High with colored indicators
- [x] Explain WHY confidence is at that level
- [x] **Node Roles**: Entry Point, Utility, Core Logic, Isolated, Adapter
- [x] Display role and confidence in CLI output

**Outcome:** Arbor is transparent, not mysterious.

---

## Phase 4: Code Reality Support (v1.4) ✅

**Real codebases aren't clean. Arbor must handle messiness.**

- [x] Dynamic call heuristics (`HeuristicsMatcher`)
- [x] Widget-tree heuristics for Flutter/Dart
- [x] Event handler detection
- [x] Callback pattern detection
- [x] Dependency injection pattern detection
- [x] `UncertainEdge` type for "possible runtime edges"

**Outcome:** Arbor works on ugly, real-world code — not just pretty examples.

---

## Phase 5: GUI v2 — Visual + Structured (v1.7) ✅

**Now that trust is solid, add carefully scoped visual features.**

- [x] Search history list with clickable buttons
- [ ] Optional graph panel (not default view)
- [ ] Collapsible call tree
- [ ] File path → click to open in editor
- [ ] "Suggested safe refactors" section

**Outcome:** GUI becomes a real productivity tool, not a gimmick.

---

## Phase 6: Workflow Integration (v1.8–v1.9) ✅

**Fit Arbor into developers' daily routines.**

- [x] PR summary generator (`arbor pr-summary`)
- [x] `arbor watch` mode: auto-refresh index on file save
- [ ] AI-friendly JSON output modes
- [ ] Editor integrations (Cursor, VS Code)
- [ ] Configurable ignore patterns

**Outcome:** Arbor becomes something people use 5× per day, not once per week.

---

## Phase 7: v2.0 Identity Lock-In 🔜

**Promise:** *"If Arbor says a change is safe, you understand why."*

### Requirements
- [x] GUI exists and functional
- [x] CLI output consistent and human-friendly
- [x] Clear confidence/uncertainty signals
- [x] Supports common real-world patterns (frameworks, widgets, async)
- [x] Caching stable for large repos
- [x] No empty or useless outputs
- [ ] Zero confusion around installation or crate name
- [x] Single blessed install path shown everywhere (README, Quickstart, CLI)
- [x] Polished documentation

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
