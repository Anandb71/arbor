param(
    [string]$Repo = "Anandb71/arbor",
    [string]$ReadmePath = "README.md",
    [int]$MaxContributors = 15
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not (Test-Path $ReadmePath)) {
    throw "README not found at path: $ReadmePath"
}

$headers = @{
    "User-Agent" = "arbor-contributors-updater"
    "Accept"     = "application/vnd.github+json"
}

$contributorsUrl = "https://api.github.com/repos/$Repo/contributors?per_page=100"
$contributors = Invoke-RestMethod -Uri $contributorsUrl -Headers $headers -Method Get

if ($null -eq $contributors) {
    $contributors = @()
}

# Remove bot accounts from display
$humanContributors = @($contributors | Where-Object { $_.type -ne "Bot" })
$total = $humanContributors.Count
$shown = @($humanContributors | Select-Object -First $MaxContributors)

$cards = @()
foreach ($c in $shown) {
    $login = [string]$c.login
    $avatar = [string]$c.avatar_url
    $profile = [string]$c.html_url

    $displayName = $login
    try {
        $user = Invoke-RestMethod -Uri "https://api.github.com/users/$login" -Headers $headers -Method Get
        if ($user.name -and $user.name.Trim().Length -gt 0) {
            $displayName = $user.name.Trim()
        }
    } catch {
        # Keep login fallback when profile lookup fails.
    }

    $card = @"
  <a href="$profile" title="$displayName" style="text-decoration:none; margin:8px; display:inline-block; text-align:center; width:88px;">
    <img src="$avatar" alt="$displayName" width="64" height="64" style="border-radius:50%;" /><br />
    <sub><b>$displayName</b></sub>
  </a>
"@
    $cards += $card.TrimEnd()
}

$more = if ($total -gt $MaxContributors) { $total - $MaxContributors } else { 0 }
$moreLine = if ($more -gt 0) { '<p align="center"><strong>+' + $more + ' more</strong></p>' } else { '' }

$generated = @"
## Contributors

<!-- CONTRIBUTORS:START -->
<p align="center">
$($cards -join "`n")
</p>
$moreLine
<!-- CONTRIBUTORS:END -->
"@

$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$readme = [System.IO.File]::ReadAllText($ReadmePath, [System.Text.Encoding]::UTF8)

$startMarker = "<!-- CONTRIBUTORS:START -->"
$endMarker = "<!-- CONTRIBUTORS:END -->"

if ($readme.Contains($startMarker) -and $readme.Contains($endMarker)) {
    $pattern = "(?s)## Contributors\s*\R\R<!-- CONTRIBUTORS:START -->.*?<!-- CONTRIBUTORS:END -->"
    $updated = [regex]::Replace($readme, $pattern, $generated.Trim())
} else {
    $updated = $readme.TrimEnd() + "`r`n`r`n---`r`n`r`n" + $generated.Trim() + "`r`n"
}

[System.IO.File]::WriteAllText($ReadmePath, $updated, $utf8NoBom)
Write-Host "Updated contributors section in $ReadmePath (total contributors: $total, shown: $($shown.Count))."
