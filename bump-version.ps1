param([string]$Path = "src-tauri")
$vf = Join-Path $Path "version.txt"
$v = (Get-Content $vf).Trim()
$p = $v -split '\.'
$p[2] = [int]$p[2] + 1
$nv = $p -join '.'

Set-Content -Path $vf -Value $nv -NoNewline

# Only bump the package version line (anchored to start of line), not dependency versions.
$tc = Get-Content (Join-Path $Path "tauri.conf.json") -Raw
$tc = $tc -replace '"version":\s*"[^"]*"', ('"version": "' + $nv + '"')
Set-Content -Path (Join-Path $Path "tauri.conf.json") -Value $tc -NoNewline

$ct = Get-Content (Join-Path $Path "Cargo.toml") -Raw
$ct = $ct -replace '(?m)^version = "[^"]*"', ('version = "' + $nv + '"')
Set-Content -Path (Join-Path $Path "Cargo.toml") -Value $ct -NoNewline

Write-Host "Bumped version: $v -> $nv"
Write-Output $nv