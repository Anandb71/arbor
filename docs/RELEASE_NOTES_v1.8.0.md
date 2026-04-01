# Arbor v1.8.0 Release Notes — "Reliability by Default"

> **Theme:** Hardening release infrastructure so every publish path is deterministic, observable, and safer under real-world CI conditions.

**Release date:** April 2026

---

## Highlights

- **Full workflow overhaul** across release, CI, publish, and utility automation
- **Branch-safe release automation** to eliminate detached-HEAD push failures
- **Improved extension publishing resilience** with icon path compatibility and stronger diagnostics
- **Cleaner release operations** with explicit secret requirements and safer conditional publishing

---

## What’s New

### 1) Release pipeline rebuilt (`.github/workflows/release.yml`)

The release pipeline was redesigned from scratch for reliability and traceability:

- Added a **prepare stage** to normalize and validate tag/version metadata
- Hardened cross-platform binary build matrix and artifact packaging
- Kept crates.io publish in dependency order with safe token-aware behavior
- Improved GitHub Release creation and asset handling
- Fixed package-manager checksum updates with:
  - explicit branch checkout (`main`)
  - commit only when files actually changed
  - safer push behavior for protected branch scenarios

### 2) Core CI & publishing workflows modernized

Rebuilt and standardized:

- `rust.yml`
- `vscode-marketplace.yml`
- `ghcr.yml`
- `npm-publish.yml`
- `flutter.yml`

Key improvements:

- Safer concurrency controls
- Modern action versions and caching strategy
- Better preflight diagnostics for publishing paths
- Clear skip behavior when optional secrets are not configured

### 3) Utility and MCP workflows rebuilt

Reworked for consistency and idempotency:

- `contributors.yml`
- `set-topics.yml`
- `arbor-action-smoke.yml`
- `mcp-integration-validate.yml`
- `mcp-release-adoption.yml`

### 4) npm wrapper publishing hardening

Addressed recurring publish failures and ownership issues:

- npm package moved to owner scope: **`@anandb71/arbor-cli`**
- Added committed cross-platform launcher (`packaging/npm/bin/arbor.js`)
- Version alignment in npm workflow now follows release tags

### 5) VS Code extension packaging hardening

Addressed recurring CI failures caused by legacy icon path lookups:

- Added icon-path compatibility fallback for packaging
- Improved extension publish workflow diagnostics to surface metadata/path issues quickly

### 6) Release docs expanded

Updated release runbook (`docs/RELEASING.md`) with:

- required/optional secrets
- npm publish expectations
- clearer workflow map for release operations

---

## Validation Performed

- `cargo check --workspace` ✅
- `cargo test --workspace` ✅
- VS Code extension compile + package (`vsce package`) ✅
- npm package dry-run (`npm pack --dry-run`) ✅

---

## Required Secrets for Full v1.8 Publishing

Configure in **GitHub → Settings → Secrets and variables → Actions**:

- `CARGO_REGISTRY_TOKEN` (required for crates.io publishing)
- `NPM_TOKEN` (required for npm wrapper publishing)
- `OVSX_PAT` (required for Open VSX publishing)
- `VSCE_PAT` (required for VS Code Marketplace publishing)

> If optional tokens are missing, related publish steps will skip safely.

---

## Upgrade Path

```bash
cargo install arbor-graph-cli --version 1.8.0
```

npm wrapper usage:

```bash
npx @anandb71/arbor-cli
```

No breaking API/runtime changes are introduced in this release; this is a reliability and release-operations hardening release.

---

## Notable Internal Commits (v1.8 branch)

- `2b35bab` — rebuild release workflow for v1.8 reliability
- `a16b44d` — rebuild core CI and publishing workflows
- `88991d4` — rebuild utility workflows and MCP automation
- `cc0ca86` — update release docs and secret requirements
- `0034b60` — align npm wrapper versioning and add committed bin launcher
