# Contributing to Arbor

We want to make Arbor the standard for code intelligence. Your help is essential to making that happen!

## 🌍 Language Bounty Board
We are aggressively expanding language support. If you know Tree-sitter, we want you!

| Language | Status | Priority | Difficulty |
|----------|--------|----------|------------|
| **TypeScript** | ✅ Stable | High | Medium |
| **Go** | ✅ Stable | High | Low |
| **Python** | ✅ Stable | High | Low |
| **Java** | ✅ Stable | Medium | High |
| **Kotlin** | ❌ Missing | Medium | High |
| **Ruby** | ❌ Missing | Low | Medium |

**Reward:** Contributors of new language parsers will be featured in our "Hall of Fame" in the README and Release Notes.

## 🛠️ How to Contribute

1.  **Fork & Clone**
    ```bash
    git clone https://github.com/YOUR_USERNAME/arbor.git
    cd arbor
    ```

2.  **Pick a Task**
    - Check [ROADMAP.md](docs/ROADMAP.md) for high-level goals.
    - Look for "Good First Issue" tags on GitHub.

3.  **Create a Branch**
    ```bash
    git checkout -b feature/cool-new-thing
    ```

4.  **Test Your Changes**
    ```bash
    arbor setup
    arbor doctor

    cargo test --workspace
    cargo fmt --all
    cargo clippy --workspace -- -D warnings
    ```

5.  **Submit a PR**
    - Describe *why* you made the change.
    - Include screenshots for UI changes.
    - Reference any relevant issues.

## 🎨 Design Philosophy
*   **Local-First:** No data leaves the user's machine.
*   **Fast:** Sub-100ms response times for queries.
*   **Trustable:** Always explain *why* suggestions are made (see `arbor refactor --why`).
*   **Easy Anywhere:** Arbor should work from any subdirectory with minimal setup friction.

## 💬 Community
Join the discussion on GitHub Issues or start a standard Github Discussion!
