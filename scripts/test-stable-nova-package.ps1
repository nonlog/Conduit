[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$PackageZip
)

$ErrorActionPreference = 'Stop'
$zipPath = (Resolve-Path -LiteralPath $PackageZip).Path
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$expected = @(
    'conduit-nova.exe',
    'Conduit.exe', 'Conduit.dll', 'Conduit.pri', 'resources.pri',
    'conduit-daemon.exe', 'conduit-send.exe',
    'Conduit.ShareTarget.msix', 'Conduit.ShareTarget.cer',
    'tools/install-windows.ps1', 'tools/uninstall-windows.ps1'
)
$stage = Join-Path ([IO.Path]::GetTempPath()) ('conduit-nova-package-qa-' + [guid]::NewGuid().ToString('N'))
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
        [IO.Compression.ZipFile]::ExtractToDirectory($zipPath, $stage)
    }
    finally { $zip.Dispose() }

    foreach ($name in @('conduit-nova.exe', 'Conduit.exe', 'conduit-daemon.exe')) {
        $path = Join-Path $stage $name
        $item = Get-Item -LiteralPath $path
        $minSize = if ($name -eq 'Conduit.exe') { 64KB } else { 1MB }
        if ($item.Length -lt $minSize) { throw "Unexpectedly small Windows executable $name" }
        $stream = [IO.File]::OpenRead($path)
        try { if ($stream.ReadByte() -ne 77 -or $stream.ReadByte() -ne 90) { throw "Not a PE executable: $name" } }
        finally { $stream.Dispose() }
    }

    # Keep the sparse MSIX identity on the WinUI executable: Nova is the default
    # shell, while Windows continues to activate the tested ShareOperation host.
    $shareZip = [IO.Compression.ZipFile]::OpenRead((Join-Path $stage 'Conduit.ShareTarget.msix'))
    try {
        $entry = $shareZip.GetEntry('AppxManifest.xml')
        if (-not $entry) { throw 'Windows Share Target is missing its AppxManifest.xml' }
        $reader = [IO.StreamReader]::new($entry.Open())
        try { [xml]$manifest = $reader.ReadToEnd() } finally { $reader.Dispose() }
        if ($manifest.Package.Identity.Name -ne 'Conduit.Desktop.ShareTarget' -or
            $manifest.Package.Applications.Application.Executable -ne 'Conduit.exe') {
            throw 'Share Target must remain bound to the WinUI compatibility host'
        }
        if ($manifest.OuterXml -notmatch 'windows.shareTarget' -or
            $manifest.OuterXml -notmatch 'StorageItems') {
            throw 'Windows file Share Target registration is incomplete'
        }
    }
    finally { $shareZip.Dispose() }

    $production = Get-Content (Join-Path $root 'windows/conduit-tauri/src-tauri/tauri.production.conf.json') -Raw | ConvertFrom-Json
    if ($production.app.windows[0].title -ne 'Conduit' -or $production.identifier -ne 'com.nonlog.conduit.desktop') {
        throw 'Production Nova GUI must not share the Preview identity or title'
    }
    $installer = Get-Content (Join-Path $stage 'tools/install-windows.ps1') -Raw
    if ($installer -notmatch 'conduit-nova\.exe' -or $installer -notmatch 'Conduit\.exe' -or
        $installer -notmatch 'ConduitShortcut' -or $installer -notmatch 'ShareTargetRegistered') {
        throw 'Stable installer must prefer Nova and retain WinUI Share Target and fallback'
    }

    [pscustomobject]@{
        Result = 'passed'
        Nova = 'conduit-nova.exe'
        ShareTarget = 'Conduit.exe (WinUI compatibility host)'
        Entries = $entries.Count
        ZipSha256 = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}
finally {
    if (Test-Path -LiteralPath $stage) {
        Remove-Item -LiteralPath $stage -Recurse -Force
    }
}
