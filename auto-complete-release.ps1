#!/usr/bin/env pwsh
# Monitor for PR merge and automatically complete the release

$maxAttempts = 120  # 10 minutes with 5-second intervals
$attempt = 0

Write-Host "Monitoring for PR merge..." -ForegroundColor Cyan

while ($attempt -lt $maxAttempts) {
    $attempt++
    
    # Check if main has the fix
    git fetch origin -q 2>&1 | Out-Null
    $mainContent = git show origin/main:.github/workflows/release.yml 2>&1 | Select-String '\(\.\[0-9\]\+\.\)'
    
    if ($mainContent -match '\(\\.\\[0-9\\]\\+\\\\\.\\[0-9\\]\\+\\)') {
        Write-Host "✓ PR merge detected! Completing release..." -ForegroundColor Green
        
        # Remove old tag
        Write-Host "Removing old v1.8.0.1 tag..."
        git tag -d v1.8.0.1 2>&1 | Out-Null
        git push origin :v1.8.0.1 2>&1 | Out-Null
        
        # Create new tag
        Write-Host "Creating new v1.8.0.1 tag from merged main..."
        git tag v1.8.0.1 origin/main
        git push origin v1.8.0.1
        
        Write-Host "✓ Release triggered! Check: https://github.com/Anandb71/arbor/actions?query=workflow:Release" -ForegroundColor Green
        exit 0
    }
    
    if ($attempt % 12 -eq 0) {
        Write-Host "Waiting... ($([Math]::Round($attempt * 5 / 60, 1)) minutes elapsed)" -ForegroundColor Yellow
    }
    
    Start-Sleep -Seconds 5
}

Write-Host "✗ Timeout: PR not merged within 10 minutes" -ForegroundColor Red
Write-Host "Please merge the PR manually: https://github.com/Anandb71/arbor/compare/main...hotfix/semver-4-part" -ForegroundColor Yellow
exit 1
