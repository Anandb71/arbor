[CmdletBinding()]
param(
    [ValidateSet("all", "claude-code", "cursor", "vscode")]
    [string]$Client = "all",

    [string]$TargetDir = (Get-Location).Path,

    [switch]$Force
)

$ErrorActionPreference = "Stop"

function Write-Step {
    param([string]$Message)
    Write-Host "[arbor-mcp-setup] $Message" -ForegroundColor Cyan
}

$scriptDir = Split-Path -Parent $PSCommandPath
$rootDir = Split-Path -Parent $scriptDir
$templateDir = Join-Path $rootDir "templates/mcp"

function Copy-Template {
    param(
        [string]$Source,
        [string]$Destination
    )

    $destFolder = Split-Path -Parent $Destination
    if (-not (Test-Path $destFolder)) {
        New-Item -ItemType Directory -Path $destFolder -Force | Out-Null
    }

    if ((Test-Path $Destination) -and -not $Force) {
        Write-Step "Skipping existing file: $Destination (use -Force to overwrite)"
        return
    }

    Copy-Item -Path $Source -Destination $Destination -Force
    Write-Step "Wrote: $Destination"
}

function Apply-Client {
    param([string]$Name)

    switch ($Name) {
        "claude-code" {
            Copy-Template -Source (Join-Path $templateDir "claude-code.project.mcp.json") -Destination (Join-Path $TargetDir ".mcp.json")
        }
        "cursor" {
            Copy-Template -Source (Join-Path $templateDir "cursor.project.mcp.json") -Destination (Join-Path $TargetDir ".cursor/mcp.json")
        }
        "vscode" {
            Copy-Template -Source (Join-Path $templateDir "vscode.project.mcp.json") -Destination (Join-Path $TargetDir ".vscode/mcp.json")
        }
        default {
            throw "Unsupported client: $Name"
        }
    }
}

switch ($Client) {
    "all" {
        Apply-Client "claude-code"
        Apply-Client "cursor"
        Apply-Client "vscode"
    }
    default {
        Apply-Client $Client
    }
}

Write-Step "Done."
Write-Step "Next: restart your client or reload MCP servers."
