# Arbor Release Runbook (Publish Everywhere)

This runbook ensures Arbor releases propagate across all distribution channels — not only GitHub Releases.

## Release channels covered

- GitHub Release assets (multi-platform CLI binaries)
- crates.io (workspace crates)
- GHCR container image (`ghcr.io/anandb71/arbor`)
- VS Code Marketplace extension
- Open VSX extension
- MCP release note enrichment snippets

## Required repository secrets

Configure these in **Settings → Secrets and variables → Actions**:

- `CARGO_REGISTRY_TOKEN` — crates.io publishing token
- `VSCE_PAT` — VS Code Marketplace publisher token (optional but recommended)
- `OVSX_PAT` — Open VSX publisher token (optional but recommended)

> Extension publishing requires at least one of `VSCE_PAT` or `OVSX_PAT`.

## Workflow map

- `.github/workflows/release.yml`
  - Trigger: tag push (`v*`)
  - Builds cross-platform CLI binaries
  - Publishes crates to crates.io
  - Creates GitHub Release and uploads assets

- `.github/workflows/ghcr.yml`
  - Trigger: GitHub Release published
  - Builds and publishes GHCR image (tag + latest)

- `.github/workflows/vscode-marketplace.yml`
  - Trigger: GitHub Release published or manual dispatch
  - Compiles extension
  - Resolves extension version from release tag (`vX.Y.Z` → `X.Y.Z`) unless manually overridden
  - Publishes packaged VSIX to VS Code Marketplace and/or Open VSX

- `.github/workflows/mcp-release-adoption.yml`
  - Trigger: GitHub Release published
  - Appends MCP quick-install snippets to release notes

## Recommended release sequence

1. Ensure `CHANGELOG.md` and release notes docs are updated.
2. Ensure workspace version and internal crate dependency versions are aligned.
3. Create and push a release tag:
   - `vX.Y.Z`
4. Wait for all workflows to complete:
   - Release
   - GHCR
   - VS Code Extension Publish
   - MCP Release Adoption Notes

## Versioning conventions

- Git tag format: `vX.Y.Z`
- Cargo crates: `X.Y.Z`
- VS Code extension package: `X.Y.Z` (derived from release tag automatically in release-triggered publish)

## Verification checklist

After release completion, verify:

- GitHub Releases contains all CLI assets
- `cargo install arbor-graph-cli --version X.Y.Z` succeeds
- `docker pull ghcr.io/anandb71/arbor:latest` succeeds
- VS Code Marketplace listing shows latest extension version
- Open VSX listing shows latest extension version
- Release notes include MCP install snippet section
