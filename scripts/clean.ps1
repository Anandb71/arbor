param(
    [switch]$Deep
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-DirSizeBytes {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return 0 }
    return (Get-ChildItem -Path $Path -Recurse -Force -ErrorAction SilentlyContinue | Measure-Object -Property Length -Sum).Sum
}

$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot

$targets = @(
    'crates/target',
    'visualizer/build',
    'visualizer/.dart_tool'
)

if ($Deep) {
    $targets += @(
        '.arbor',
        'visualizer/.flutter-plugins',
        'visualizer/.flutter-plugins-dependencies'
    )
}

Write-Host "Arbor workspace cleanup started in: $repoRoot"

$freedBytes = 0
foreach ($target in $targets) {
    if (Test-Path $target) {
        $sizeBefore = Get-DirSizeBytes -Path $target
        Remove-Item -Path $target -Recurse -Force
        $freedBytes += $sizeBefore
        Write-Host ("Removed {0} ({1:N2} MB)" -f $target, ($sizeBefore / 1MB))
    }
    else {
        Write-Host "Skipped $target (not found)"
    }
}

Write-Host ("Cleanup complete. Freed approximately {0:N2} GB." -f ($freedBytes / 1GB))
Write-Host "Tip: run 'cargo test --workspace' afterwards to rebuild only what you need."
