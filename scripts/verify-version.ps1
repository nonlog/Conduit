[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$ExpectedVersion
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

$cargo = Get-Content -LiteralPath (Join-Path $repoRoot 'Cargo.toml') -Raw
$android = Get-Content -LiteralPath (Join-Path $repoRoot 'android\app\build.gradle.kts') -Raw
$desktop = Get-Content -LiteralPath (Join-Path $repoRoot 'windows\conduit-ui\Conduit.csproj') -Raw

$cargoMatch = [regex]::Match($cargo, '(?m)^version\s*=\s*"([^"]+)"')
$androidMatch = [regex]::Match($android, '(?m)^\s*versionName\s*=\s*"([^"]+)"')
$desktopMatch = [regex]::Match($desktop, '<ApplicationDisplayVersion>([^<]+)</ApplicationDisplayVersion>')

if (-not $cargoMatch.Success -or -not $androidMatch.Success -or -not $desktopMatch.Success) {
    throw 'Could not read one or more project versions'
}

$versions = [ordered]@{
    Cargo = $cargoMatch.Groups[1].Value
    Android = $androidMatch.Groups[1].Value
    Desktop = $desktopMatch.Groups[1].Value
}

$bad = $versions.GetEnumerator() | Where-Object { $_.Value -ne $ExpectedVersion }
if ($bad) {
    $details = ($versions.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ', '
    throw "Release tag version $ExpectedVersion does not match project versions: $details"
}

[pscustomobject]@{
    ExpectedVersion = $ExpectedVersion
    Cargo = $versions.Cargo
    Android = $versions.Android
    Desktop = $versions.Desktop
}
