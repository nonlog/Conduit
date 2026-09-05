[CmdletBinding()]
param(
    [string]$ManifestPath = (Join-Path $PSScriptRoot '..\windows\conduit-ui\ShareTargetPackage\AppxManifest.xml'),
    [Parameter(Mandatory)]
    [string]$OutputPackage
)

$ErrorActionPreference = 'Stop'
$manifest = (Resolve-Path -LiteralPath $ManifestPath).Path
$output = [IO.Path]::GetFullPath($OutputPackage)

$makeAppx = Get-Command MakeAppx.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -First 1
if ([string]::IsNullOrWhiteSpace($makeAppx)) {
    $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $makeAppx = Get-ChildItem -LiteralPath $kits -Filter MakeAppx.exe -File -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match '[\\/]x64[\\/]MakeAppx\.exe$' } |
        Sort-Object FullName -Descending |
        Select-Object -ExpandProperty FullName -First 1
}
if ([string]::IsNullOrWhiteSpace($makeAppx) -or -not (Test-Path -LiteralPath $makeAppx -PathType Leaf)) {
    throw 'MakeAppx.exe was not found in the Windows SDK'
}

$stage = Join-Path ([IO.Path]::GetTempPath()) "conduit-share-target-$PID"
try {
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    Copy-Item -LiteralPath $manifest -Destination (Join-Path $stage 'AppxManifest.xml') -Force
    $outputDir = [IO.Path]::GetDirectoryName($output)
    New-Item -ItemType Directory -Force -Path $outputDir | Out-Null
    Remove-Item -LiteralPath $output -Force -ErrorAction SilentlyContinue

    & $makeAppx pack /d $stage /p $output /nv /o
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $output -PathType Leaf)) {
        throw "MakeAppx failed to create $output"
    }

    [pscustomobject]@{
        Package = $output
        SizeBytes = (Get-Item -LiteralPath $output).Length
        Sha256 = (Get-FileHash -LiteralPath $output -Algorithm SHA256).Hash.ToLowerInvariant()
    }
}
finally {
    Remove-Item -LiteralPath $stage -Recurse -Force -ErrorAction SilentlyContinue
}
