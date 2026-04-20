# Arbor v2.0.1 Release Notes — "Patch Stability & Automation Fixes"

> **Theme:** Fast, low-risk patch release to stabilize release automation and PR-impact reporting after v2.0.0.

**Release date:** April 20, 2026

---

## Highlights

- **PR Bot command path fixed**
  - Switched from invalid subcommand path to real CLI-driven report generation via:
    - `arbor diff . --json`
  - Added PR commit-range awareness using `ARBOR_DIFF_BASE` and `ARBOR_DIFF_HEAD`.

- **PR comment rendering fixed**
  - JSON report output now renders as proper fenced Markdown in PR comments.

- **Regression protection added**
  - Added integration test coverage for ranged diff behavior in `arbor-cli` so commit-range reporting does not regress.

- **Contributors automation hardened**
  - `contributors.yml` now skips PR creation if contributor block is unchanged.
  - Uses `github.token` consistently in checkout/PR creation path.
  - Contributors script now uses Bearer auth and handles transient GitHub API failures gracefully.

---

## Version Alignment

Release-facing versions were aligned to **2.0.1** across:

- Cargo workspace package version
- Homebrew formula
- Scoop manifest
- npm wrapper package
- VS Code extension `package.json` / `package-lock.json`

---

## Validation

Validated during release prep with:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features`
- `cargo test --workspace --verbose`
- `cargo test -p arbor-graph-cli --test diff_command_integration --verbose`

---

## Upgrade

```bash
cargo install arbor-graph-cli --version 2.0.1
```

---

## Notes

This is a patch release focused on correctness and workflow reliability. No intentional breaking changes were introduced.
