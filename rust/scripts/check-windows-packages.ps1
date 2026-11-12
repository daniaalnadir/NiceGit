# Checks the Windows downloads made by package-windows.ps1 on this machine: the zip holds the app
# and its notices, the app starts, and the installer installs, upgrades in place and uninstalls
# cleanly. Needs an administrator shell, as on GitHub's Windows runners.
#
# Example: pwsh rust/scripts/check-windows-packages.ps1 -Version 2.1.0 -Arch arm64
param(
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][ValidateSet('x64', 'arm64')][string]$Arch
)

$ErrorActionPreference = 'Stop'
$dist = Join-Path (Split-Path -Parent $PSScriptRoot) 'dist'
$name = "NiceGit-$Version-windows-$Arch"

function Assert-Machine([string]$exe) {
    # IMAGE_FILE_MACHINE_AMD64 is 0x8664 and IMAGE_FILE_MACHINE_ARM64 is 0xAA64.
    $bytes = [IO.File]::ReadAllBytes($exe)
    $pe = [BitConverter]::ToInt32($bytes, 0x3C)
    $machine = [BitConverter]::ToUInt16($bytes, $pe + 4)
    $expected = @{ x64 = 0x8664; arm64 = 0xAA64 }[$Arch]
    if ($machine -ne $expected) { throw ('{0} is built for machine 0x{1:X4}, not {2}' -f $exe, $machine, $Arch) }
}

function Assert-Starts([string]$exe) {
    # A window that stays open for 15 seconds has got past start-up, including the renderer.
    $process = Start-Process -FilePath $exe -PassThru
    Start-Sleep -Seconds 15
    if ($process.HasExited) { throw "$exe exited during start-up with code $($process.ExitCode)" }
    Stop-Process -Id $process.Id -Force
    Write-Host "$exe started"
}

$zip = Join-Path $dist "$name.zip"
$unzipped = Join-Path ([IO.Path]::GetTempPath()) "$name-check"
Remove-Item -Recurse -Force $unzipped -ErrorAction SilentlyContinue
Expand-Archive -Path $zip -DestinationPath $unzipped
foreach ($file in 'nicegit.exe', 'LICENSE', 'README.md', 'THIRD_PARTY_LICENSES.txt') {
    if (-not (Test-Path (Join-Path $unzipped $file))) { throw "$zip lacks $file" }
}
Assert-Machine (Join-Path $unzipped 'nicegit.exe')
Assert-Starts (Join-Path $unzipped 'nicegit.exe')

$msi = Join-Path $dist "$name.msi"
$installed = Join-Path $env:ProgramFiles 'NiceGit\nicegit.exe'
$shortcut = Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs\NiceGit.lnk'

function Invoke-Msi([string[]]$arguments, [string]$log) {
    $process = Start-Process msiexec.exe -ArgumentList ($arguments + @('/qn', '/l*v', $log)) -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        Get-Content $log -Tail 60
        throw "msiexec $($arguments -join ' ') failed with code $($process.ExitCode)"
    }
}

Invoke-Msi @('/i', "`"$msi`"") (Join-Path $dist 'install.log')
foreach ($file in $installed, $shortcut, (Join-Path $env:ProgramFiles 'NiceGit\THIRD_PARTY_LICENSES.txt')) {
    if (-not (Test-Path $file)) { throw "the installer did not create $file" }
}
Assert-Machine $installed
Assert-Starts $installed
$entry = Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*' |
    Where-Object { $_.DisplayName -eq 'NiceGit' }
if (-not $entry) { throw 'NiceGit is not listed under installed apps' }
Write-Host "Installed NiceGit $($entry.DisplayVersion)"

# Installing the same package again repairs it in place rather than adding a second copy.
Invoke-Msi @('/i', "`"$msi`"") (Join-Path $dist 'reinstall.log')
$entries = @(Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*' |
    Where-Object { $_.DisplayName -eq 'NiceGit' })
if ($entries.Count -ne 1) { throw "NiceGit is listed $($entries.Count) times after installing again" }

Invoke-Msi @('/x', "`"$msi`"") (Join-Path $dist 'uninstall.log')
foreach ($file in $installed, $shortcut) {
    if (Test-Path $file) { throw "uninstalling left $file behind" }
}
Remove-Item -Force (Join-Path $dist '*.log')
Write-Host "$name.zip and $name.msi are good"
