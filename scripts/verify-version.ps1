[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$ExpectedVersion
)

$ErrorActionPreference = 'Stop'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path

$cargo = Get-Content -LiteralPath (Join-Path $repoRoot 'Cargo.toml') -Raw
$android = Get-Content -LiteralPath (Join-Path $repoRoot 'android\app\build.gradle.kts') -Raw
$runeCargo = Get-Content -LiteralPath (Join-Path $repoRoot 'windows\conduit-rune\Cargo.toml') -Raw
$shareCargo = Get-Content -LiteralPath (Join-Path $repoRoot 'windows\conduit-share-target\Cargo.toml') -Raw
$shareManifest = Get-Content -LiteralPath (Join-Path $repoRoot 'windows\conduit-share-target\ShareTargetPackage\AppxManifest.xml') -Raw

$cargoMatch = [regex]::Match($cargo, '(?m)^version\s*=\s*"([^"]+)"')
$androidMatch = [regex]::Match($android, '(?m)^\s*versionName\s*=\s*"([^"]+)"')
$runeMatch = [regex]::Match($runeCargo, '(?m)^version\s*=\s*"([^"]+)"')
$shareCargoMatch = [regex]::Match($shareCargo, '(?m)^version\.workspace\s*=\s*true')
$shareMatch = [regex]::Match($shareManifest, '\bVersion="(\d+\.\d+\.\d+\.\d+)"')

if (-not $cargoMatch.Success -or -not $androidMatch.Success -or -not $runeMatch.Success -or
    -not $shareCargoMatch.Success -or -not $shareMatch.Success) {
    throw 'Could not read one or more project versions'
}

$shareTargetVersion = $shareMatch.Groups[1].Value
if ($shareTargetVersion -notmatch ('^' + [regex]::Escape($ExpectedVersion) + '\.\d+$')) {
    throw "Share Target package version $shareTargetVersion must start with $ExpectedVersion and include a package revision"
}

$versions = [ordered]@{
    Cargo = $cargoMatch.Groups[1].Value
    Android = $androidMatch.Groups[1].Value
    Rune = $runeMatch.Groups[1].Value
    ShareTarget = $shareTargetVersion
}

$bad = $versions.GetEnumerator() | Where-Object {
    $_.Key -ne 'ShareTarget' -and $_.Value -ne $ExpectedVersion
}
if ($bad) {
    $details = ($versions.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ', '
    throw "Release tag version $ExpectedVersion does not match project versions: $details"
}

[pscustomobject]@{
    ExpectedVersion = $ExpectedVersion
    Cargo = $versions.Cargo
    Android = $versions.Android
    Rune = $versions.Rune
    ShareTarget = $versions.ShareTarget
}
