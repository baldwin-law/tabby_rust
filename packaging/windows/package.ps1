# Build a portable Windows zip.
#
# Usage:
#   pwsh -File packaging/windows/package.ps1 -Binary target/release/tabby-rust.exe `
#        -Version 0.1.0 -OutDir dist [-Arch x86_64]

param(
    [Parameter(Mandatory = $true)][string]$Binary,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [string]$Arch = "x86_64"
)

$ErrorActionPreference = "Stop"

$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$stage = Join-Path $OutDir "tabby-rust-$Version-windows-$Arch"
$zip = Join-Path $OutDir "tabby-rust-$Version-windows-$Arch.zip"

if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Path $stage | Out-Null

Copy-Item $Binary (Join-Path $stage "tabby-rust.exe")
Copy-Item (Join-Path $root "README.md") $stage
Copy-Item (Join-Path $root "LICENSE") $stage

if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path (Join-Path $stage "*") -DestinationPath $zip -CompressionLevel Optimal

Write-Host "wrote $zip"
