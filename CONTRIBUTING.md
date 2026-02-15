# Contributing always to Arbor

We want to make Arbor the standard for code intelligence. Your help is essential to making that happen!

## 🌍 Language Bounty Board
We are aggressively expanding language support. If you know Tree-sitter, we want you!

| Language | Status | Priority | Difficulty |
|----------|--------|----------|------------|
| **TypeScript** | ✅ Beta | High | Medium |
| **Go** | 🚧 Planned | High | Low |
| **Python** | 🚧 Planned | High | Low |
| **Java** | ❌ Missing | Medium | High |
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
    cargo test --all
    cargo fmt --all
    cargo clippy
    ```

5.  **Submit a PR**
    - Describe *why* you made the change.
    - Include screenshots for UI changes.
    - Reference any relevant issues.

## 🎨 Design Philosophy
*   **Local-First:** No data leaves the user's machine.
*   **Fast:** Sub-100ms response times for queries.
*   **Trustable:** Always explain *why* suggestions are made (see `arbor refactor --why`).

## 💬 Community
Join the discussion on GitHub Issues or start a standard Github Discussion!
