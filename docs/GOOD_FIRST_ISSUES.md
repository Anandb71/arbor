# Good First Issues for Arbor

These are beginner-friendly issues for new contributors.

---

## 1. Add Kotlin Parser Support

**Labels**: `good first issue`, `enhancement`, `parser`

**Description**: Add Tree-sitter-based Kotlin parser to support Android/JVM projects.

**What you'll learn**:

- Tree-sitter grammar integration
- AST node extraction patterns
- Arbor's parser architecture

**Files to modify**:

- `crates/arbor-core/Cargo.toml` (add tree-sitter-kotlin)
- `crates/arbor-core/src/languages/kotlin.rs` (new)
- `crates/arbor-core/src/languages/mod.rs` (register)

**Getting started**:

1. Look at `rust.rs` or `python.rs` as examples
2. Install tree-sitter-kotlin
3. Map AST nodes to Arbor NodeKinds

---

## 2. Add Dark/Light Theme Toggle to Visualizer

**Labels**: `good first issue`, `visualizer`, `enhancement`

**Description**: Add a button to toggle between dark and light themes in the visualizer.

**What you'll learn**:

- Flutter theming
- Riverpod state management
- UI design patterns

**Files to modify**:

- `visualizer/lib/core/theme.dart`
- `visualizer/lib/views/forest_view.dart`

---

## 3. Add `--output-json` Flag to CLI Commands

**Labels**: `good first issue`, `cli`, `enhancement`

**Description**: Add JSON output option for CLI commands for scripting/automation.

**What you'll learn**:

- Clap argument parsing
- Serde JSON serialization
- CLI UX patterns

**Files to modify**:

- `crates/arbor-cli/src/main.rs`
- `crates/arbor-cli/src/commands.rs`

---

## 4. Add File Filter to Visualizer Search

**Labels**: `good first issue`, `visualizer`, `enhancement`

**Description**: Allow filtering nodes by file path in the search bar.

**What you'll learn**:

- Flutter text field handling
- Node filtering logic
- State management

---

## 5. Improve Error Messages for Missing Dependencies

**Labels**: `good first issue`, `dx`, `enhancement`

**Description**: Make error messages more helpful when Tree-sitter or Flutter is missing.

**What you'll learn**:

- Rust error handling
- User experience for CLIs

---

## How to Pick an Issue

1. Comment "I'd like to work on this" on the issue
2. Fork the repo and create a branch
3. Follow CONTRIBUTING.md
4. Open a PR linking the issue

Need help? Open a Discussion or ask in the issue!
