# Arbor Roadmap: Path to v2.0

> **North Star:** Arbor is the tool you run *before* refactoring, not after something breaks.

## Theme
**From "useful tool" to "default pre-refactor safety net".**

---

## Phase 0: Stabilize the Hero (Now → v1.3.x) 🔒
**Goal:** Make `arbor refactor` boringly reliable.

- [x] **v1.3.0 The Cache Update**: Faster indexing, smarter resolution.
- [ ] **Output Polishing**: Refine wording based on confusion reports.
- [ ] **Better Fallbacks**: Improve suggestions if target not found.
- [ ] **Clarity**: Make "why this is safe/risky" explicit.

*Rule: No new commands. No architecture changes.*

---

## Phase 1: Confidence Signals (v1.4)
**Theme:** "Can I trust this?"

Responding to: *"I rely on tests / types / compiler."*

- [ ] **Confidence Score**: Soft, explainable risk rating (Low/Medium/High).
- [ ] **Explainer**: "This looks safe structurally. Tests still recommended for behavior."
- [ ] **Better Role Detection**: Explicitly identify Core Domain vs Adapters vs Utilities.

*Outcome: Arbor feels like a pre-flight checklist, not a judge.*

---

## Phase 2: Reality Handling (v1.5)
**Theme:** "Real code isn't perfect."

- [ ] **Dynamic Edge Awareness**: Best-effort heuristics for callbacks, framework hooks (e.g., Flutter widgets).
- [ ] **Uncertainty Surfacing**: "This dependency *may* exist at runtime."
- [ ] **Dead/Suspicious Node Detection**: Framed as investigation ("No callers found"), not accusation.

*Outcome: Arbor feels honest about limitations — building trust.*

---

## Phase 3: Workflow Fit (v1.6–1.7)
**Theme:** "Fits how people actually work."

- [ ] **Pre-Refactor Mode**: `arbor refactor auth --mode cautious` (Verbose, conservative).
- [ ] **Refactor Notes**: Generate markdown summaries pasteable into PR descriptions.
- [ ] **IDE QoL**: Jump-to-file, line numbers, copyable paths in CLI output.

*Outcome: Arbor becomes part of the ritual, not a one-off tool.*

---

## Phase 4: Teaching the Mental Model (v1.8)
**Theme:** "Explain the system, not just the change."

- [ ] **Call Path Narratives**: "Request enters here → flows through layers → exits here."
- [ ] **Layer Detection**: Controller, Service, Domain, Infrastructure.
- [ ] **Architecture Smells**: Gentle observations ("This function has unrelated callers").

*Outcome: Arbor teaches people how their code works.*

---

## Phase 5: The Contract (v2.0)
**Theme:** "I trust this before I touch code."

v2.0 isn't about more features. It's a promise: **"If Arbor says this is safe, you understand why."**

**Criteria:**
- 1. `arbor refactor` never feels empty.
- 2. Output always explains reasoning.
- 3. Uncertainty is explicit.
- 4. Works on ugly real-world code.
- 5. Helps even when tests/types exist.

---

## What is NOT on the Roadmap (Yet)
*To keep focus, these stay out until after v2:*
- Fancy visualizer overhauls
- LSP parity with IDEs
- AI automated refactoring
- Full runtime analysis
