[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$RustReleaseDir,

    [Parameter(Mandatory)]
    [string]$OutputZip,

    [string]$ShareTargetPackage,

    [string]$ShareTargetCertificate,

    [Parameter(Mandatory)]
    [string]$ShareTargetExecutable,

    [Parameter(Mandatory)]
    [string]$RuneExecutable
)

$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$rust = (Resolve-Path $RustReleaseDir).Path
$shareTarget = (Resolve-Path $ShareTargetExecutable).Path
$output = [IO.Path]::GetFullPath($OutputZip)
$stage = Join-Path ([IO.Path]::GetDirectoryName($output)) 'windows-package'

$requiredRust = @('conduit-daemon.exe', 'conduit-send.exe')
if (-not (Test-Path -LiteralPath $shareTarget -PathType Leaf)) {
    throw "Missing Rust Share Target executable: $shareTarget"
}
foreach ($name in $requiredRust) {
    if (-not (Test-Path -LiteralPath (Join-Path $rust $name) -PathType Leaf)) {
        throw "Missing Rust release binary: $(Join-Path $rust $name)"
    }
}

if (Test-Path -LiteralPath $stage) {
    Remove-Item -LiteralPath $stage -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $stage | Out-Null

# Runtime state is never package payload. If a local/build publish directory happens to contain a
# data folder, excluding it here prevents an overlay from replacing Scoop's persisted data junction.
Remove-Item -LiteralPath (Join-Path $stage 'data') -Recurse -Force -ErrorAction SilentlyContinue

Copy-Item -LiteralPath $shareTarget -Destination (Join-Path $stage 'conduit-share-target.exe') -Force

foreach ($name in $requiredRust) {
    Copy-Item -LiteralPath (Join-Path $rust $name) -Destination (Join-Path $stage $name) -Force
}

if (-not [string]::IsNullOrWhiteSpace($RuneExecutable)) {
    $rune = (Resolve-Path -LiteralPath $RuneExecutable -ErrorAction Stop).Path
    if ((Get-Item -LiteralPath $rune).Length -lt 1MB) {
        throw 'Conduit Rune executable is unexpectedly small'
    }
    $stableRune = Join-Path $stage 'conduit-rune.exe'
    Copy-Item -LiteralPath $rune -Destination $stableRune -Force
    if ((Get-FileHash -LiteralPath $rune -Algorithm SHA256).Hash -ne
        (Get-FileHash -LiteralPath $stableRune -Algorithm SHA256).Hash) {
        throw 'Rune executable changed while packaging'
    }
}

$toolsDir = Join-Path $stage 'tools'
$assetsDir = Join-Path $stage 'assets'
New-Item -ItemType Directory -Force -Path $toolsDir, $assetsDir | Out-Null
Copy-Item -LiteralPath (Join-Path $repoRoot 'windows\conduit-daemon\tools\install-windows.ps1') -Destination $toolsDir -Force
Copy-Item -LiteralPath (Join-Path $repoRoot 'windows\conduit-daemon\tools\configure-lan-firewall.ps1') -Destination $toolsDir -Force
Copy-Item -LiteralPath (Join-Path $repoRoot 'windows\conduit-daemon\tools\uninstall-windows.ps1') -Destination $toolsDir -Force
foreach ($name in @('conduit-icon.ico', 'conduit-icon.png', 'conduit-icon-light.ico', 'conduit-icon-light.png', 'conduit-icon-dark.ico', 'conduit-icon-dark.png', 'conduit-explorer-light.ico', 'conduit-explorer-dark.ico')) {
    Copy-Item -LiteralPath (Join-Path $repoRoot "windows\conduit-daemon\assets\$name") -Destination $assetsDir -Force
}
Copy-Item -LiteralPath (Join-Path $repoRoot 'README.md') -Destination $stage -Force

if (-not [string]::IsNullOrWhiteSpace($ShareTargetPackage)) {
    $sharePackage = (Resolve-Path -LiteralPath $ShareTargetPackage).Path
    Copy-Item -LiteralPath $sharePackage -Destination (Join-Path $stage 'Conduit.ShareTarget.msix') -Force
}
if (-not [string]::IsNullOrWhiteSpace($ShareTargetCertificate)) {
    $shareCertificate = (Resolve-Path -LiteralPath $ShareTargetCertificate).Path
    Copy-Item -LiteralPath $shareCertificate -Destination (Join-Path $stage 'Conduit.ShareTarget.cer') -Force
}
if (-not [string]::IsNullOrWhiteSpace($RuneExecutable)) {
    foreach ($name in @('conduit-share-target.exe', 'Conduit.ShareTarget.msix', 'Conduit.ShareTarget.cer', 'conduit-rune.exe', 'conduit-daemon.exe', 'conduit-send.exe')) {
        if (-not (Test-Path -LiteralPath (Join-Path $stage $name) -PathType Leaf)) {
            throw "Incomplete stable Windows package: missing $name"
        }
    }
}
if (-not [string]::IsNullOrWhiteSpace($RuneExecutable)) {
    if (-not (Test-Path -LiteralPath (Join-Path $stage 'conduit-rune.exe') -PathType Leaf)) {
        throw 'Stable Rune desktop was not included in the package'
    }
}

# Fail closed rather than accidentally ship a retired frontend from a reused staging directory.
foreach ($name in @('conduit-nova.exe', 'conduit-tauri-preview.exe')) {
    if (Test-Path -LiteralPath (Join-Path $stage $name)) { throw "Retired frontend in package: $name" }
}

# Debug symbols are not required by the portable/Scoop runtime.
Get-ChildItem -LiteralPath $stage -Recurse -File -Filter '*.pdb' | Remove-Item -Force

$outputDir = [IO.Path]::GetDirectoryName($output)
New-Item -ItemType Directory -Force -Path $outputDir | Out-Null
if (Test-Path -LiteralPath $output) {
    Remove-Item -LiteralPath $output -Force
}
Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $output -CompressionLevel Optimal

$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $output).Hash.ToLowerInvariant()
[pscustomobject]@{
    Package = $output
    Sha256 = $hash
    SizeBytes = (Get-Item -LiteralPath $output).Length
}
