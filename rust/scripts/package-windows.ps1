# Packages a built nicegit.exe into the Windows downloads, in rust/dist:
#   NiceGit-<version>-windows-<arch>.zip   the portable app (also what winget installs)
#   NiceGit-<version>-windows-<arch>.msi   the installer
#
# Run from anywhere after `cargo build --release` and rust/scripts/third-party-licenses.sh. Sign
# nicegit.exe before running this, so both downloads carry the signed app. The installer needs
# WiX Toolset 6 and its UI extension:
#   dotnet tool install --global wix --version 6.0.2
#   wix extension add --global WixToolset.UI.wixext/6.0.2
#
# Example: pwsh rust/scripts/package-windows.ps1 -Version 2.1.0 -Arch arm64
param(
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][ValidateSet('x64', 'arm64')][string]$Arch
)

$ErrorActionPreference = 'Stop'
$rust = Split-Path -Parent $PSScriptRoot
$root = Split-Path -Parent $rust
$dist = Join-Path $rust 'dist'
$exe = Join-Path $rust 'target/release/nicegit.exe'
$notices = Join-Path $dist 'THIRD_PARTY_LICENSES.txt'
foreach ($file in @($exe, $notices)) {
    if (-not (Test-Path $file)) { throw "$file is missing; build NiceGit and its licence file first" }
}

$name = "NiceGit-$Version-windows-$Arch"
$stage = Join-Path ([IO.Path]::GetTempPath()) $name
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $stage, $dist | Out-Null
Copy-Item $exe, $notices, (Join-Path $root 'LICENSE'), (Join-Path $root 'README.md') -Destination $stage

$zip = Join-Path $dist "$name.zip"
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip -Force
Write-Host "Created $zip"

# Windows Installer versions are numbers only, so a pre-release such as 2.1.0-rc.1 installs as 2.1.0.
$msiVersion = ($Version -split '-')[0]

# The licence page shows rich text; turn the plain MIT licence into a minimal RTF document.
$escaped = (Get-Content -Raw (Join-Path $root 'LICENSE')).Replace('\', '\\').Replace('{', '\{').Replace('}', '\}')
$paragraphs = ($escaped -replace "`r", '') -split "`n" -join "\par`n"
$rtf = Join-Path ([IO.Path]::GetTempPath()) 'NiceGit-license.rtf'
Set-Content -Path $rtf -Encoding ascii -Value "{\rtf1\ansi\deff0{\fonttbl{\f0 Segoe UI;}}\fs18`n$paragraphs}"

$msi = Join-Path $dist "$name.msi"
& wix build (Join-Path $rust 'packaging/windows/nicegit.wxs') `
    -arch $Arch `
    -ext WixToolset.UI.wixext `
    -d "Version=$msiVersion" `
    -d "SourceDir=$stage" `
    -d "IconFile=$(Join-Path $rust 'assets/NiceGit.ico')" `
    -d "LicenseRtf=$rtf" `
    -o $msi
if ($LASTEXITCODE -ne 0) { throw 'wix build failed' }
Remove-Item -Force (Join-Path $dist "$name.wixpdb") -ErrorAction SilentlyContinue
Write-Host "Created $msi"
