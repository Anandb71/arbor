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

1. **Fork & Clone**
    ```bash
    git clone https://github.com/YOUR_USERNAME/arbor.git
    cd arbor
    ```

2. **Pick a Task**
    - Check [ROADMAP.md](docs/ROADMAP.md) for high-level goals.
    - Look for "Good First Issue" tags on GitHub.

3. **Create a Branch**
    ```bash
    git checkout -b feature/cool-new-thing
    ```

## 🌿 Branch Strategy

To keep releases maintainable and avoid cross-version confusion:

- `main` → active development for next minor/major
- `release/v1.5` → maintenance-only patches for 1.5.x
- `release/v1.6` → 1.6 feature work and stabilization

Rules of thumb:

1. New features go to `main` or the current release branch (for example `release/v1.6`).
2. Bug fixes that must ship to existing users are cherry-picked/backported to the matching maintenance branch (for example `release/v1.5`).
3. Avoid landing new-version features in older maintenance branches.

4. **Test Your Changes**
    ```bash
    arbor setup
    arbor doctor

    cargo test --workspace
    cargo fmt --all
    cargo clippy --workspace -- -D warnings
    ```

5. **Submit a PR**
    - Describe *why* you made the change.
    - Include screenshots for UI changes.
    - Reference any relevant issues.

## ✅ PR Quality Checklist (2026)

Before requesting review:

- [ ] Branch targets the correct release line (`release/v1.5` vs `release/v1.6` vs `main`)
- [ ] Tests pass locally (`cargo test --workspace`)
- [ ] Lint and formatting pass (`cargo clippy --workspace -- -D warnings`, `cargo fmt --all`)
- [ ] Docs are updated for user-facing changes (README + relevant docs under `docs/`)
- [ ] New CLI behavior has tests (unit and/or integration where meaningful)
- [ ] PR summary explains risk, rollout plan, and any migration impact

## 🧾 Commit and Release Notes Guidance

- Prefer clear, scoped commit messages (Conventional Commits style is welcome: `feat:`, `fix:`, `docs:`, `test:`).
- Add user-visible behavior changes to `CHANGELOG.md` under `[Unreleased]`.
- For risky behavior changes (diff heuristics, analysis confidence), include explicit edge cases in PR notes.
- Keep generated files out of commits (`target/`, extension `out/`, local logs, and temporary exports).
- Before opening PRs, squash noisy local fixup commits into a small, reviewable set.

## 🎨 Design Philosophy
*   **Local-First:** No data leaves the user's machine.
*   **Fast:** Sub-100ms response times for queries.
*   **Trustable:** Always explain *why* suggestions are made (see `arbor refactor --why`).
*   **Easy Anywhere:** Arbor should work from any subdirectory with minimal setup friction.

## 💬 Community
Join the discussion on GitHub Issues or start a standard Github Discussion!
