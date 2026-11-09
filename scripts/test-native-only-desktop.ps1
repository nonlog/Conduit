[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
function Read-Source([string]$Path) {
    Get-Content -LiteralPath (Join-Path $root $Path) -Raw
}
foreach ($path in @('windows/conduit-tauri', '.github/workflows/tauri-preview.yml')) {
    if (Test-Path -LiteralPath (Join-Path $root $path)) {
        throw "Retired frontend remains in source: $path"
    }
}
$workflow = Read-Source '.github/workflows/build.yml'
if ($workflow -match 'setup-bun|bun run|conduit-tauri|NovaExecutable') {
    throw 'Windows build must not install or build the retired WebView frontend'
}
if ($workflow -notmatch 'test-native-only-desktop.ps1' -or $workflow -notmatch 'RuneExecutable') {
    throw 'Native desktop packaging and regression gate are required'
}
$packager = Read-Source 'scripts/package-windows.ps1'
if ($packager -match '\$NovaExecutable' -or $packager -notmatch '\[Parameter\(Mandatory\)\]\s*\[string\]\$RuneExecutable') {
    throw 'Packager must require Rune and have no Nova input'
}
$rune = Read-Source 'windows/conduit-rune/src/main.rs'
$bridge = Read-Source 'windows/conduit-rune/src/bridge.rs'
$tray = Read-Source 'windows/conduit-daemon/src/tray.rs'
$shareHost = Read-Source 'windows/conduit-share-target/src/main.rs'
$send = (Read-Source 'windows/conduit-daemon/src/bin/conduit-send.rs') -split '#\[cfg\(test\)\]', 2
foreach ($source in @($bridge, $tray, $shareHost, $send[0])) {
    if ($source -match 'open_nova_ui|conduit-nova\.exe|conduit-tauri-preview\.exe') {
        throw 'A runtime launcher still selects the retired frontend'
    }
}
if ($rune -match 'open_nova_ui|open_legacy_ui|fn stat\(') {
    throw 'Rune must not expose alternative desktop buttons or dashboard statistic cards'
}
if ($shareHost -notmatch 'ActivationKind::ShareTarget' -or
    $shareHost -notmatch 'GetStorageItemsAsync' -or
    $shareHost -notmatch 'conduit-send.exe') {
    throw 'The native Rust share host must activate, receive StorageItems and send through conduit-send'
}
if (Test-Path (Join-Path $root 'windows/conduit-ui/Conduit.csproj')) {
    throw 'The retired WinUI compatibility project remains in source'
}
$installer = Read-Source 'windows/conduit-daemon/tools/install-windows.ps1'
if ($installer -match '\$novaExe|DefaultDesktop\s*=.*Nova') {
    throw 'Installer still advertises Nova as a desktop'
}
if ($installer -notmatch "'conduit-share-target\.exe'" -or
    $installer -notmatch "'Conduit\.ShareTarget\.msix'" -or
    $installer -notmatch "'data'") {
    throw 'The install allowlist must preserve native runtime and user data while excluding WinUI payload'
}
Write-Output 'NATIVE_SOURCE_CHECK_PASSED: Rune desktop, Rust Share Target, no WinUI project, persisted data preserved'
