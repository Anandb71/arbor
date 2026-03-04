[CmdletBinding()]
param(
    [string]$Version = "latest",
    [string]$InstallDir = "$HOME\.arbor\bin",
    [switch]$DryRun,
    [switch]$Force
)

$ErrorActionPreference = "Stop"

function Write-Step {
    param([string]$Message)
    Write-Host "[arbor-install] $Message" -ForegroundColor Cyan
}

function Resolve-AssetName {
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture

    switch ($arch) {
        "X64" { return "arbor-windows-x64.exe" }
        default {
            throw "Unsupported Windows architecture '$arch'. Expected X64."
        }
    }
}

function Get-ReleaseMeta {
    param([string]$Version)

    $base = "https://api.github.com/repos/Anandb71/arbor/releases"
    $url = if ($Version -eq "latest") { "$base/latest" } else { "$base/tags/$Version" }

    Write-Step "Resolving release metadata from GitHub API ($Version)..."
    return Invoke-RestMethod -Uri $url -Headers @{ "User-Agent" = "arbor-install-script" }
}

function Add-ToPathIfMissing {
    param([string]$Dir)

    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($null -eq $userPath) { $userPath = "" }

    $parts = $userPath.Split(';', [System.StringSplitOptions]::RemoveEmptyEntries)
    if ($parts -contains $Dir) {
        Write-Step "Install directory already present in user PATH."
        return
    }

    $newPath = if ([string]::IsNullOrWhiteSpace($userPath)) { $Dir } else { "$userPath;$Dir" }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    Write-Step "Added '$Dir' to your user PATH. Restart terminal to pick it up."
}

try {
    $assetName = Resolve-AssetName
    $release = Get-ReleaseMeta -Version $Version

    $asset = $release.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1
    if (-not $asset) {
        throw "Could not find asset '$assetName' in release '$($release.tag_name)'."
    }

    $targetDir = [System.IO.Path]::GetFullPath($InstallDir)
    $targetExe = Join-Path $targetDir "arbor.exe"

    if ($DryRun) {
        Write-Step "Dry run enabled."
        Write-Host "Would install: $($asset.browser_download_url)" -ForegroundColor Yellow
        Write-Host "Target path : $targetExe" -ForegroundColor Yellow
        exit 0
    }

    if (-not (Test-Path $targetDir)) {
        Write-Step "Creating install directory: $targetDir"
        New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
    }

    if ((Test-Path $targetExe) -and -not $Force) {
        Write-Step "Existing arbor binary found. Use -Force to overwrite."
        Write-Host "Existing: $targetExe" -ForegroundColor Yellow
        exit 0
    }

    $tmpFile = Join-Path ([System.IO.Path]::GetTempPath()) "arbor-installer-$([Guid]::NewGuid()).exe"
    Write-Step "Downloading $assetName ..."
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $tmpFile -UseBasicParsing

    Write-Step "Installing to $targetExe"
    Move-Item -Path $tmpFile -Destination $targetExe -Force

    Add-ToPathIfMissing -Dir $targetDir

    Write-Step "Install complete."
    Write-Host "Run: arbor --version" -ForegroundColor Green
}
catch {
    Write-Host "Install failed: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}
