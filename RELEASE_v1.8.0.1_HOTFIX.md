# v1.8.0.1 Release Hotfix Instructions

## Problem
The v1.8.0.1 release workflow failed with:
```
Invalid semver tag/version resolved: TAG=v1.8.0.1 VERSION=1.8.0.1
```

## Root Cause
The release.yml workflow only accepts 3-part semver versions (e.g. v1.8.0) but v1.8.0.1 has 4 parts.

## Solution
The fix is on branch `hotfix/semver-4-part` which updates the regex to accept 4-part versions:
- OLD: `^[0-9]+\.[0-9]+\.[0-9]+$`
- NEW: `^[0-9]+\.[0-9]+\.[0-9]+(\.[0-9]+)?$`

## Steps to Complete Release

### Step 1: Merge the Hotfix PR
1. Visit: https://github.com/Anandb71/arbor/compare/main...hotfix/semver-4-part
2. Click "Create Pull Request"
3. Click "Merge pull request"
4. Confirm the merge

### Step 2: Re-trigger the Release Workflow
After the PR is merged, run these commands:

```powershell
cd c:\Users\anand\Repos\arbor
git fetch origin
git tag -d v1.8.0.1
git push origin :v1.8.0.1
git tag v1.8.0.1 origin/main
git push origin v1.8.0.1
```

This will:
- Remove the old v1.8.0.1 tag that failed
- Create a new v1.8.0.1 tag pointing to the merged main commit
- Push the tag to GitHub (triggers release workflow automatically)

### Step 3: Monitor the Release
Visit: https://github.com/Anandb71/arbor/actions?query=workflow:Release

The release workflow should now:
- ✅ Pass semver validation with v1.8.0.1
- ✅ Build for all 5 targets
- ✅ Publish to crates.io
- ✅ Create GitHub Release
- ✅ Trigger downstream: GHCR, npm, VSCode Marketplace, MCP adoption

## Expected Outcomes
- Crates published to crates.io with version 1.8.0.1
- npm package @anandb71/arbor-cli@1.8.0.1 published
- VS Code extension version 1.8.0.1 
- Docker image pushed to GHCR
- GitHub Release created with assets

## Files Changed in Hotfix
- `.github/workflows/release.yml`: Updated semver regex (line ~46) 
  from `^[0-9]+\.[0-9]+\.[0-9]+$` to `^[0-9]+\.[0-9]+\.[0-9]+(\.[0-9]+)?$`

That's it! All other workflows are already correct and ready.
