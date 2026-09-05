[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$UiPublishDir,

    [Parameter(Mandatory)]
    [string]$RustReleaseDir,

    [Parameter(Mandatory)]
    [string]$OutputZip,

    [string]$ShareTargetPackage
)

$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$ui = (Resolve-Path $UiPublishDir).Path
$rust = (Resolve-Path $RustReleaseDir).Path
$output = [IO.Path]::GetFullPath($OutputZip)
$stage = Join-Path ([IO.Path]::GetDirectoryName($output)) 'windows-package'

$requiredUi = Join-Path $ui 'Conduit.exe'
$requiredRust = @('conduit-daemon.exe', 'conduit-send.exe')
if (-not (Test-Path -LiteralPath $requiredUi -PathType Leaf)) {
    throw "Missing published desktop executable: $requiredUi"
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

# The WinUI publish is self-contained, so the whole publish directory is part of the package.
Copy-Item -Path (Join-Path $ui '*') -Destination $stage -Recurse -Force

# Runtime state is never package payload. If a local/build publish directory happens to contain a
# data folder, excluding it here prevents an overlay from replacing Scoop's persisted data junction.
Remove-Item -LiteralPath (Join-Path $stage 'data') -Recurse -Force -ErrorAction SilentlyContinue

foreach ($name in $requiredRust) {
    Copy-Item -LiteralPath (Join-Path $rust $name) -Destination (Join-Path $stage $name) -Force
}

$toolsDir = Join-Path $stage 'tools'
$assetsDir = Join-Path $stage 'assets'
New-Item -ItemType Directory -Force -Path $toolsDir, $assetsDir | Out-Null
Copy-Item -LiteralPath (Join-Path $repoRoot 'windows\conduit-daemon\tools\install-windows.ps1') -Destination $toolsDir -Force
Copy-Item -LiteralPath (Join-Path $repoRoot 'windows\conduit-daemon\tools\uninstall-windows.ps1') -Destination $toolsDir -Force
foreach ($name in @('conduit-icon.ico', 'conduit-icon.png', 'conduit-icon-light.ico', 'conduit-icon-light.png', 'conduit-icon-dark.ico', 'conduit-icon-dark.png', 'conduit-explorer-light.ico', 'conduit-explorer-dark.ico')) {
    Copy-Item -LiteralPath (Join-Path $repoRoot "windows\conduit-daemon\assets\$name") -Destination $assetsDir -Force
}
Copy-Item -LiteralPath (Join-Path $repoRoot 'README.md') -Destination $stage -Force

if (-not [string]::IsNullOrWhiteSpace($ShareTargetPackage)) {
    $sharePackage = (Resolve-Path -LiteralPath $ShareTargetPackage).Path
    Copy-Item -LiteralPath $sharePackage -Destination (Join-Path $stage 'Conduit.ShareTarget.msix') -Force
}

# Debug symbols are not required by the portable/Scoop runtime and make the WinUI package much larger.
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
