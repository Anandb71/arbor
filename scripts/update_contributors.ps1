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
$contributorsPage = "https://github.com/$Repo/graphs/contributors"

$cards = @()
foreach ($c in $shown) {
    $login = [string]$c.login
    $avatar = [string]$c.avatar_url
    $profile = [string]$c.html_url

    $card = @"
    <a href="$profile" title="$login" style="text-decoration:none; margin:6px; display:inline-block;">
        <img src="$avatar" alt="$login" width="72" height="72" loading="lazy" style="border-radius:50%; border:2px solid #30363d; box-sizing:border-box;" />
  </a>
"@
    $cards += $card.TrimEnd()
}

$more = if ($total -gt $MaxContributors) { $total - $MaxContributors } else { 0 }
$moreLine = if ($more -gt 0) { '<p align="center"><strong>+' + $more + ' more</strong></p>' } else { '' }
$summaryLine = '<p align="center"><sub><strong>' + $total + ' contributors</strong> | <a href="' + $contributorsPage + '">View all</a></sub></p>'

$generated = @"
## Contributors

<!-- CONTRIBUTORS:START -->
<p align="center">
$($cards -join "`n")
</p>
$summaryLine
$moreLine
<!-- CONTRIBUTORS:END -->
"@

$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$readme = [System.IO.File]::ReadAllText($ReadmePath, [System.Text.Encoding]::UTF8)

$startMarker = "<!-- CONTRIBUTORS:START -->"
$endMarker = "<!-- CONTRIBUTORS:END -->"

if ($readme.Contains($startMarker) -and $readme.Contains($endMarker)) {
    $pattern = "(?s)## Contributors\s*\r?\n\r?\n<!-- CONTRIBUTORS:START -->.*?<!-- CONTRIBUTORS:END -->"
    $updated = [regex]::Replace($readme, $pattern, $generated.Trim())
} else {
    $updated = $readme.TrimEnd() + "`r`n`r`n---`r`n`r`n" + $generated.Trim() + "`r`n"
}

[System.IO.File]::WriteAllText($ReadmePath, $updated, $utf8NoBom)
Write-Host "Updated contributors section in $ReadmePath (total contributors: $total, shown: $($shown.Count))."
