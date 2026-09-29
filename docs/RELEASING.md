# Arbor Release Runbook (Publish Everywhere)

This runbook ensures Arbor releases propagate across all distribution channels — not only GitHub Releases.

## Release channels covered

- GitHub Release assets (multi-platform CLI binaries)
- crates.io (workspace crates)
- GHCR container image (`ghcr.io/anandb71/arbor`)
- VS Code Marketplace extension
- Open VSX extension
- Scoop manifest (`packaging/scoop/arbor.json`, installed by URL)
- Homebrew formula (`packaging/homebrew/arbor.rb`, kept current; no tap is published yet)
- npm wrapper (`packaging/npm/`)
- MCP release note enrichment snippets

## Required repository secrets

Configure these in **Settings → Secrets and variables → Actions**:

- `CARGO_REGISTRY_TOKEN` — crates.io publishing token. crates.io tokens can expire: an expired one fails the publish with a 403, and the job now says so instead of skipping quietly
- `VSCE_PAT` — VS Code Marketplace publisher token (optional but recommended)
- `OVSX_PAT` — Open VSX publisher token (optional but recommended)
- `NPM_TOKEN` — npm publish token for `@anandb71/arbor-cli` wrapper (optional but recommended)

> Extension publishing requires at least one of `VSCE_PAT` or `OVSX_PAT`.
>
> npm wrapper publishing requires `NPM_TOKEN`.

Also enable **Settings → Actions → General → Allow GitHub Actions to create and approve pull requests**. Without it the release workflow still pushes the `chore/manifest-checksums-vX.Y.Z` branch, but cannot open its pull request; open it by hand from the link in the job's error.

## Workflow map

- `.github/workflows/release.yml`
  - Trigger: tag push (`v*`)
  - Checks that the tag matches `Cargo.toml`, `agent-card.json`, `packaging/npm/package.json` and `extensions/arbor-vscode/package.json`
  - Builds cross-platform CLI binaries (5 targets)
  - Creates GitHub Release and uploads assets
  - Publishes crates to crates.io (a failure is reported on the job but does not fail the run)
  - Writes the new version and checksums into the Homebrew formula and Scoop manifest, and opens a pull request with them

- `.github/workflows/ghcr.yml`
  - Trigger: the Release workflow completing successfully, or manual dispatch
  - Builds and publishes GHCR image (tag + latest)

- `.github/workflows/vscode-marketplace.yml`
  - Trigger: the Release workflow completing successfully, or manual dispatch
  - Compiles extension
  - Resolves extension version from release tag (`vX.Y.Z` → `X.Y.Z`) unless manually overridden
  - Publishes packaged VSIX to VS Code Marketplace and/or Open VSX

- `.github/workflows/mcp-release-adoption.yml`
  - Trigger: GitHub Release published
  - Appends MCP quick-install snippets to release notes

- `.github/workflows/arbor-pr-bot.yml`
  - Trigger: Pull request events
  - Runs `analyze-impact` and posts blast-radius governance comment
  - Gracefully posts fallback context when markdown report generation fails

- `.github/workflows/npm-publish.yml`
  - Trigger: the Release workflow completing successfully, or manual dispatch
  - Publishes npm wrapper package from `packaging/npm/`

## Recommended release sequence

1. Ensure `CHANGELOG.md` and `docs/RELEASE_NOTES_vX.Y.Z.md` are updated.
2. Bump the workspace version, the internal crate dependency versions, `Cargo.lock`, `agent-card.json`, `packaging/npm/package.json` and `extensions/arbor-vscode/package.json` (and its lockfile). Leave `packaging/homebrew` and `packaging/scoop` alone: the release workflow writes them once the assets and their checksums exist.
3. Create and push a release tag:
   - `vX.Y.Z`
4. Wait for all workflows to complete:
   - Release
   - GHCR
   - Publish NPM Wrapper
   - Publish VS Code Extension
   - MCP Release Adoption Notes
5. Merge the `chore: update package manifests for vX.Y.Z` pull request.

A Release run whose crates.io or manifest job failed still concludes successfully, so the other channels publish. Check those two jobs for error annotations anyway: re-run `Publish crates to crates.io` after fixing the token, and open the manifest pull request by hand if the workflow could not.

GHCR, npm and the VS Code extension publish only after a successful Release run. If one was skipped, run its workflow manually with the tag.

## Versioning conventions

- Git tag format: `vX.Y.Z`
- Cargo crates: `X.Y.Z`
- VS Code extension package: `X.Y.Z` (derived from release tag automatically in release-triggered publish)

## Verification checklist

After release completion, verify:

- GitHub Releases contains all CLI assets
- `cargo install arbor-graph-cli --version X.Y.Z` succeeds (or `cargo search arbor-graph-cli` shows X.Y.Z)
- `npm view @anandb71/arbor-cli version` shows X.Y.Z
- `scoop install https://raw.githubusercontent.com/Anandb71/arbor/main/packaging/scoop/arbor.json` installs X.Y.Z once the manifest pull request is merged
- `docker pull ghcr.io/anandb71/arbor:latest` succeeds
- VS Code Marketplace listing shows latest extension version
- Open VSX listing shows latest extension version
- Release notes include MCP install snippet section
- Arbor PR Bot posts impact comments on new PRs (or explicit fallback comment with run URL if analysis fails)
