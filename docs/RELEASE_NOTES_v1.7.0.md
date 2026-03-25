# Arbor v1.7.0 Release Notes — "Distribution & Reach"

> **Theme:** Making Arbor available everywhere — every package manager, every editor, every CI pipeline.

**Release date:** March 25, 2026

---

## Highlights

- **Automated Cross-Platform Release Pipeline** — Tag-push triggers 5-target binary builds, crates.io publishing, GitHub Releases, GHCR images, and VS Code Marketplace/Open VSX publishing
- **Package Manager Distribution** — Homebrew formula, Scoop manifest, and npm wrapper for zero-friction install on any platform
- **Enhanced VS Code Extension** — 8 commands (up from 3), quick-pick command menu, impact analysis from editor, git diff preview, walkthrough onboarding
- **Community & Discoverability** — GitHub Sponsors funding, academic citation metadata (CITATION.cff), improved badges and topics

---

## What's New

### Automated Release Workflow (`release.yml`)

The missing keystone workflow is now in place. Pushing a `vX.Y.Z` tag automatically:

1. Builds CLI binaries for 5 targets (Linux x86_64/aarch64, macOS x86_64/aarch64, Windows x86_64)
2. Publishes all workspace crates to crates.io in dependency order
3. Creates a GitHub Release with all binary assets
4. Triggers downstream workflows: GHCR image, VS Code Marketplace, Open VSX, MCP release notes

### Package Manager Support

Install Arbor without a Rust toolchain:

```bash
# Homebrew (macOS/Linux)
brew install Anandb71/tap/arbor

# Scoop (Windows)
scoop bucket add arbor https://github.com/Anandb71/arbor
scoop install arbor

# npm (any platform)
npx @arbor-graph/cli

# Docker
docker pull ghcr.io/anandb71/arbor:latest
```

### VS Code Extension Enhancements

New commands available in the command palette and editor context menu:

- **Arbor: Analyze Impact** — Run `arbor refactor` on the symbol under cursor
- **Arbor: Git Diff Impact** — Preview blast radius of uncommitted changes
- **Arbor: Show Index Status** — Display graph health and stats
- **Arbor: Re-index Workspace** — Rebuild the code graph with progress UI
- **Arbor: Command Palette** — Quick-pick menu for all Arbor actions (`Ctrl+Shift+R`)

New settings: `arbor.autoIndex`, `arbor.maxBlastRadius`

### Community & Discoverability

- `FUNDING.yml` — GitHub Sponsors enabled
- `CITATION.cff` — Academic citation metadata
- README badges for crates.io, GitHub Release, GHCR

---

## Cleanup

- Removed duplicate `vscode-publish.yml` workflow (superseded by `vscode-marketplace.yml`)
- Removed stale `package-lock.json`, `crates/Cargo.lock`, `crates/test_output.txt`
- Updated Dockerfile to Rust 1.85 with OCI labels and git support
- Updated docker-compose.yml (removed deprecated `version` key, added bridge service)
- Expanded `.gitignore` coverage

---

## Upgrade Path

```bash
cargo install arbor-graph-cli --version 1.7.0
```

No breaking changes from v1.6.x. All existing workflows and MCP integrations continue to work.
