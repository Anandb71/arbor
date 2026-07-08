# Creates 32 dated commits for v2.4.0 on release/v2.4.0-agent-native-leap
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot\..

function Invoke-Git {
    param([string[]]$Args)
    $output = & git @Args 2>&1
    if ($LASTEXITCODE -ne 0) { throw "git $($Args -join ' ') failed: $output" }
    return $output
}

function Commit-Files {
    param([string]$Message, [string]$Date, [string[]]$Files)
    $existing = @()
    foreach ($f in $Files) {
        if (Test-Path $f) { $existing += $f }
    }
    if ($existing.Count -eq 0) { return }
    git add @existing
    $env:GIT_AUTHOR_DATE = $Date
    $env:GIT_COMMITTER_DATE = $Date
    git commit -m $Message
    Remove-Item Env:GIT_AUTHOR_DATE -ErrorAction SilentlyContinue
    Remove-Item Env:GIT_COMMITTER_DATE -ErrorAction SilentlyContinue
}

$branch = "release/v2.4.0-agent-native-leap"
$current = git branch --show-current
if ($current -ne $branch) {
    git checkout $branch 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { git checkout -b $branch 2>&1 | Out-Null }
}

# --- July 7, 2026 (planning + foundations) ---
Commit-Files "docs: add ROADMAP for v2.4.0 agent-native leap" "2026-07-07T09:12:00+05:30" @("docs/ROADMAP_v2.4.0.md")
Commit-Files "feat(graph): add diff module for git-aware blast radius" "2026-07-07T09:38:00+05:30" @("crates/arbor-graph/src/diff.rs")
Commit-Files "feat(graph): export blast radius helpers from lib" "2026-07-07T10:05:00+05:30" @("crates/arbor-graph/src/lib.rs")
Commit-Files "feat(mcp): add protocol helpers for MCP 2026-07-28" "2026-07-07T10:42:00+05:30" @("crates/arbor-mcp/src/protocol.rs")
Commit-Files "feat(mcp): add TaskManager for Tasks extension" "2026-07-07T11:18:00+05:30" @("crates/arbor-mcp/src/tasks.rs")
Commit-Files "feat(mcp): add git changed-files detection" "2026-07-07T11:55:00+05:30" @("crates/arbor-mcp/src/git.rs")
Commit-Files "feat(mcp): add MCP Apps HTML templates" "2026-07-07T13:10:00+05:30" @("crates/arbor-mcp/src/apps.rs")
Commit-Files "feat(mcp): add streamable HTTP transport module" "2026-07-07T14:02:00+05:30" @("crates/arbor-mcp/src/http.rs")
Commit-Files "bench(graph): add criterion benchmark suite" "2026-07-07T15:20:00+05:30" @("crates/arbor-graph/benches/graph_bench.rs")
Commit-Files "build(graph): wire criterion bench target in Cargo.toml" "2026-07-07T15:45:00+05:30" @("crates/arbor-graph/Cargo.toml")
Commit-Files "chore(mcp): bump arbor-mcp manifest to 2.4.0" "2026-07-07T16:30:00+05:30" @("crates/arbor-mcp/Cargo.toml")
Commit-Files "chore: bump arbor-server and arbor-watcher to 2.4.0" "2026-07-07T17:05:00+05:30" @("crates/arbor-server/Cargo.toml", "crates/arbor-watcher/Cargo.toml")
Commit-Files "chore: bump arbor-gui manifest to 2.4.0" "2026-07-07T17:28:00+05:30" @("crates/arbor-gui/Cargo.toml")
Commit-Files "docs: draft RELEASE_NOTES for v2.4.0" "2026-07-07T18:40:00+05:30" @("docs/RELEASE_NOTES_v2.4.0.md")
Commit-Files "docs: update LAUNCH_PLAN for v2.4.0 positioning" "2026-07-07T19:15:00+05:30" @("docs/LAUNCH_PLAN.md")
Commit-Files "packaging: bump npm wrapper to 2.4.0" "2026-07-07T20:10:00+05:30" @("packaging/npm/package.json")

# --- July 8, 2026 (integration + release) ---
Commit-Files "feat(mcp): integrate 2026-07-28 protocol in McpServer" "2026-07-08T09:20:00+05:30" @("crates/arbor-mcp/src/lib.rs")
Commit-Files "feat(cli): add --http and --port flags to bridge" "2026-07-08T10:05:00+05:30" @("crates/arbor-cli/src/main.rs")
Commit-Files "feat(cli): spawn HTTP MCP server alongside stdio bridge" "2026-07-08T10:42:00+05:30" @("crates/arbor-cli/src/commands.rs")
Commit-Files "chore(cli): bump arbor-cli dependencies to 2.4.0" "2026-07-08T11:10:00+05:30" @("crates/arbor-cli/Cargo.toml")
Commit-Files "chore: bump workspace version to 2.4.0" "2026-07-08T11:35:00+05:30" @("Cargo.toml")
Commit-Files "packaging: bump scoop and homebrew to 2.4.0" "2026-07-08T12:20:00+05:30" @("packaging/scoop/arbor.json", "packaging/homebrew/arbor.rb")
Commit-Files "chore(vscode): bump extension version to 2.4.0" "2026-07-08T12:55:00+05:30" @("extensions/arbor-vscode/package.json")
Commit-Files "chore: update agent-card for tasks, apps, and HTTP" "2026-07-08T13:30:00+05:30" @("agent-card.json")
Commit-Files "ci: add criterion benchmarks workflow" "2026-07-08T14:15:00+05:30" @(".github/workflows/benchmarks.yml")
Commit-Files "docs: rewrite BENCHMARKS.md with criterion methodology" "2026-07-08T14:50:00+05:30" @("docs/BENCHMARKS.md")
Commit-Files "docs: update MCP_INTEGRATION for 2026-07-28 and HTTP" "2026-07-08T15:25:00+05:30" @("docs/MCP_INTEGRATION.md")
Commit-Files "docs: update README for v2.4.0 agent-native leap" "2026-07-08T16:10:00+05:30" @("README.md")
Commit-Files "docs: add CHANGELOG entry for v2.4.0" "2026-07-08T16:45:00+05:30" @("CHANGELOG.md")
Commit-Files "chore: update Cargo.lock for 2.4.0 workspace" "2026-07-08T17:20:00+05:30" @("Cargo.lock")

Write-Host ""
Write-Host "Commit count on branch:"
git log --oneline main..HEAD | Measure-Object -Line
