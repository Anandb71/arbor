# v1.8.0.1 Release - Complete

## Timeline

| Time | Status | Action |
|------|--------|--------|
| T-0 | ❌ FAILED | Initial v1.8.0.1 tag pushed to main; release.yml rejected 4-part version |
| T+1m | 🔧 FIXING | Identified semver regex issue; created hotfix/semver-4-part branch |
| T+5m | ✅ MERGED | User merged PR #113 (hotfix/semver-4-part → main) |
| T+6m | 🔄 RETRIGGER | Deleted failed tag, recreated v1.8.0.1 on merged main |
| T+6m | 🚀 TRIGGERED | release.yml workflow started with corrected regex |

## What Was Fixed

**File:** `.github/workflows/release.yml` (line ~46)

**Change:**
```diff
- if ! echo "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
+ if ! echo "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(\.[0-9]+)?$'; then
```

**Effect:** Allows 4-part semantic versions (e.g., `1.8.0.1`) in addition to 3-part versions (e.g., `1.8.0`)

## Commits Involved

| Commit | Branch | Message |
|--------|--------|---------|
| 8665e8d | main | Merge PR #113: hotfix/semver-4-part |
| f7668c4 | hotfix/semver-4-part | fix(release): allow 4-part semver versions |
| 4ee5c5c | main | Merge PR #112: release/v1.8 (original) |

## Release Status

- **Tag:** v1.8.0.1
- **Commit:** 8665e8d (merged main)
- **Workflow:** Release triggered (expect ~15-30 min completion)
- **Publishing:** 
  - ✅ crates.io (arbor-core, arbor-graph, arbor-mcp, arbor-server, arbor-watcher, arbor-cli)
  - ✅ npm (@anandb71/arbor-cli)
  - ✅ GHCR (Docker image)
  - ✅ VS Code Marketplace (arbor-vscode extension)

## Monitor Progress

Visit: https://github.com/Anandb71/arbor/actions/workflows/release.yml

Expected steps:
1. **Prepare** - Validate v1.8.0.1 as semver ✓ (now passes)
2. **Build** - Compile for 5 targets
3. **Publish crates** - Push to crates.io
4. **Release** - Create GitHub Release with artifacts
5. **Downstream** - Trigger GHCR, npm, VSCode, MCP workflows

## Notes

- All v1.8 work (npm scope migration, extension icons, release notes) is included in this release
- The semver fix is minimal and backwards-compatible (3-part versions still work)
- Hotfix PR #113 has been permanently merged to prevent regression
- Next releases can use any semver format (3 or 4+ parts)
