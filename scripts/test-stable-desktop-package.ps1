[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$PackageZip
)

$ErrorActionPreference = 'Stop'
$zipPath = (Resolve-Path -LiteralPath $PackageZip).Path
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$expected = @(
    'conduit-rune.exe', 'conduit-share-target.exe',
    'conduit-daemon.exe', 'conduit-send.exe',
    'Conduit.ShareTarget.msix', 'Conduit.ShareTarget.cer',
    'tools/install-windows.ps1', 'tools/uninstall-windows.ps1'
)
$stage = Join-Path ([IO.Path]::GetTempPath()) ('conduit-rune-package-qa-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $stage | Out-Null
try {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($zipPath)
    try {
        $entries = @($zip.Entries | ForEach-Object { $_.FullName.Replace('\', '/') })
        foreach ($name in $expected) {
            if ($entries -cnotcontains $name) { throw "Stable package missing $name" }
        }
        if (@($entries | Where-Object { $_ -match '(^|/)data/(identity\.bin|peer\.txt|transfers\.tsv|config\.txt)$' }).Count -gt 0) {
            throw 'Package must never contain a persisted device identity or history'
        }
        if (@($entries | Where-Object { $_ -match '(^|/)(conduit-nova|conduit-tauri-preview)\.exe$' }).Count -gt 0) {
            throw 'Retired Nova/Tauri frontend must not be packaged'
        }
        if (@($entries | Where-Object {
                $_ -match '\.(dll|mui|pri|deps\.json|runtimeconfig\.json)$' -or
                $_ -match '(^|/)Microsoft\.WindowsAppSDK' -or
                $_ -match '(^|/)Microsoft\.WinUI'
            }).Count -gt 0) {
            throw 'Legacy WinUI/.NET/WebView payload must not be packaged'
        }
        [IO.Compression.ZipFile]::ExtractToDirectory($zipPath, $stage)
    }
    finally { $zip.Dispose() }

    foreach ($name in @('conduit-rune.exe', 'conduit-share-target.exe', 'conduit-daemon.exe')) {
        $path = Join-Path $stage $name
        $item = Get-Item -LiteralPath $path
        $minSize = if ($name -eq 'conduit-share-target.exe') { 64KB } else { 1MB }
        if ($item.Length -lt $minSize) { throw "Unexpectedly small Windows executable $name" }
        $stream = [IO.File]::OpenRead($path)
        try { if ($stream.ReadByte() -ne 77 -or $stream.ReadByte() -ne 90) { throw "Not a PE executable: $name" } }
        finally { $stream.Dispose() }
    }

    # Verify the actual Windows GUI subsystem. A console-subsystem UI would
    # flash a terminal whenever the tray activates it.
    $runeBytes = [IO.File]::ReadAllBytes((Join-Path $stage 'conduit-rune.exe'))
    $header = [BitConverter]::ToInt32($runeBytes, 0x3c)
    $subsystem = [BitConverter]::ToUInt16($runeBytes, $header + 92)
    if ($subsystem -ne 2) {
        throw "The Rune desktop must use Windows GUI subsystem 2, got $subsystem"
    }

    # The sparse MSIX identity activates the native Rust Share Target host.
    $shareZip = [IO.Compression.ZipFile]::OpenRead((Join-Path $stage 'Conduit.ShareTarget.msix'))
    try {
        $entry = $shareZip.GetEntry('AppxManifest.xml')
        if (-not $entry) { throw 'Windows Share Target is missing its AppxManifest.xml' }
        $reader = [IO.StreamReader]::new($entry.Open())
        try { [xml]$manifest = $reader.ReadToEnd() } finally { $reader.Dispose() }
        if ($manifest.Package.Identity.Name -ne 'Conduit.Desktop.ShareTarget' -or
            $manifest.Package.Applications.Application.Executable -ne 'conduit-share-target.exe') {
            throw 'Share Target must remain bound to the native Rust host'
        }
        if ($manifest.OuterXml -notmatch 'windows.shareTarget' -or
            $manifest.OuterXml -notmatch 'StorageItems') {
            throw 'Windows file Share Target registration is incomplete'
        }
    }
    finally { $shareZip.Dispose() }

    $installer = Get-Content (Join-Path $stage 'tools/install-windows.ps1') -Raw
    if ($installer -notmatch 'conduit-rune\.exe' -or
        $installer -notmatch 'conduit-share-target\.exe' -or
        $installer -notmatch 'ConduitShortcut' -or $installer -notmatch 'ShareTargetRegistered') {
        throw 'Stable installer must select Rune and preserve the Windows Share Target host'
    }

    [pscustomobject]@{
        Result = 'passed'
        DefaultDesktop = 'conduit-rune.exe'
        WebViewFrontend = $false
        ShareTarget = 'conduit-share-target.exe (Rust host)'
        Entries = $entries.Count
        ZipSha256 = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}
finally {
    if (Test-Path -LiteralPath $stage) {
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
}
