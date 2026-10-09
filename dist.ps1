# Builds a release and copies what it needs to run into dist: rasterwarp.exe and the
# FFmpeg DLLs build.rs puts next to it. Run from the repo root:
#   powershell -ExecutionPolicy Bypass -File .\dist.ps1
# dist\ is gitignored. Files already in it are overwritten, never deleted.

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build --release failed" }

$release = Join-Path $PSScriptRoot 'target\release'
$dist = Join-Path $PSScriptRoot 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null

Copy-Item (Join-Path $release 'rasterwarp.exe') $dist -Force
$dlls = Get-ChildItem (Join-Path $release '*.dll')
if ($dlls.Count -eq 0) { throw "no FFmpeg DLLs in $release; check FFMPEG_DIR" }
$dlls | Copy-Item -Destination $dist -Force

Write-Host "dist ready:"
Get-ChildItem $dist | Format-Table Name, Length -AutoSize
