$ErrorActionPreference = 'Stop'
# Chrome DevTools fix for the DSH web profile.
# IMPORTANT: run this ONLY after fully quitting DSH Desktop, so no host
# process guards/reverts the patch file while this edits it.

$p = if ($env:DSH_PROFILE_PATCH) {
  $env:DSH_PROFILE_PATCH
} else {
  Join-Path ($env:USERPROFILE ?? $env:HOME ?? '.') '.dsh\profiles\web\cordis.patch.yml'
}
if (-not (Test-Path $p)) { Write-Error "patch not found: $p"; exit 1 }

$t = [System.IO.File]::ReadAllText($p)

# Backup (do this unconditionally)
$bak = $p + '.bak-mcp-fix-' + (Get-Date -Format 'yyyyMMdd-HHmmss')
[System.IO.File]::WriteAllText($bak, $t, (New-Object System.Text.UTF8Encoding($false)))
Write-Output "Backup: $bak"

# Unconditional replace: both variants -> mcp
$t = $t.Replace("chrome-devtools-extension@latest", "chrome-devtools-mcp@latest")
$t = $t.Replace("chrome-devtools-extensions@latest", "chrome-devtools-mcp@latest")
$t = $t.Replace("chrome-devtools-mcp-extension@latest", "chrome-devtools-mcp@latest")

[System.IO.File]::WriteAllText($p, $t, (New-Object System.Text.UTF8Encoding($false)))

# Verify by scanning raw bytes with Get-Content
Write-Output "--- verification (line filter) ---"
Get-Content $p | Select-String -Pattern 'chrome-devtools-(mcp|extension)' | ForEach-Object { "  L" + $_.LineNumber + ": " + $_.Line }
