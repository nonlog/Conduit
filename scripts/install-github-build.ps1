[CmdletBinding()]
param(
    [long]$RunId,
    [string]$Branch = 'master',
    [string]$Repo = 'nonlog/Conduit',
    [string]$AdbSerial,
    [switch]$SkipWindows,
    [switch]$SkipAndroid
)

$ErrorActionPreference = 'Stop'

function Require-Command([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "Required command is not available: $Name"
    }
}

Require-Command gh
if (-not $SkipWindows) { Require-Command scoop }
if (-not $SkipAndroid) { Require-Command adb }

if (-not $RunId) {
    $json = gh run list --repo $Repo --workflow build.yml --branch $Branch --status success --limit 1 --json databaseId,headSha,url
    if ($LASTEXITCODE -ne 0) { throw 'Could not query GitHub Actions runs' }
    $runs = $json | ConvertFrom-Json
    if (-not $runs -or $runs.Count -eq 0) {
        throw "No successful Build workflow run found for $Repo branch $Branch"
    }
    $RunId = [long]$runs[0].databaseId
}

$temp = Join-Path ([IO.Path]::GetTempPath()) "conduit-github-$RunId-$PID"
New-Item -ItemType Directory -Force -Path $temp | Out-Null

try {
    if (-not $SkipWindows) {
        $windowsDownload = Join-Path $temp 'windows-download'
        $windowsStage = Join-Path $temp 'windows-stage'
        gh run download $RunId --repo $Repo --name conduit-windows-x64 --dir $windowsDownload
        if ($LASTEXITCODE -ne 0) { throw 'Could not download the Windows GitHub Actions artifact' }

        $zip = Get-ChildItem -LiteralPath $windowsDownload -File -Filter '*.zip' | Select-Object -First 1
        if (-not $zip) { throw 'Windows Actions artifact did not contain a zip package' }
        Expand-Archive -LiteralPath $zip.FullName -DestinationPath $windowsStage -Force

        $installDir = (& scoop prefix conduit).Trim()
        if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($installDir) -or -not (Test-Path -LiteralPath $installDir -PathType Container)) {
            throw 'Conduit is not installed through Scoop on this machine'
        }

        # Development artifacts are installed on Log only. Preserve the Scoop-managed data junction;
        # the GitHub package contains no data directory, so an overlay updates only program files.
        Get-Process Conduit, conduit-daemon -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 500
        Copy-Item -Path (Join-Path $windowsStage '*') -Destination $installDir -Recurse -Force

        $installer = Join-Path $installDir 'tools\install-windows.ps1'
        if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) {
            throw "GitHub package is missing the installer: $installer"
        }
        & $installer -SourceDir $installDir -InstallDir $installDir
    }

    if (-not $SkipAndroid) {
        $androidDownload = Join-Path $temp 'android-download'
        gh run download $RunId --repo $Repo --name conduit-android-debug --dir $androidDownload
        if ($LASTEXITCODE -ne 0) { throw 'Could not download the Android GitHub Actions artifact' }

        $apk = Get-ChildItem -LiteralPath $androidDownload -Recurse -File -Filter '*.apk' | Select-Object -First 1
        if (-not $apk) { throw 'Android Actions artifact did not contain an APK' }

        $serial = $AdbSerial
        if ([string]::IsNullOrWhiteSpace($serial)) {
            $devices = @(adb devices | Select-String '^\S+\s+device$' | ForEach-Object { ($_ -split '\s+')[0] })
            if ($devices.Count -ne 1) {
                throw "Expected exactly one online ADB device; found $($devices.Count). Pass -AdbSerial when needed."
            }
            $serial = $devices[0]
        }

        adb -s $serial install -r $apk.FullName
        if ($LASTEXITCODE -ne 0) { throw "ADB install failed for $serial" }
    }

    [pscustomobject]@{
        Repository = $Repo
        RunId = $RunId
        WindowsInstalled = -not $SkipWindows
        AndroidInstalled = -not $SkipAndroid
    }
}
finally {
    if (Test-Path -LiteralPath $temp) {
        Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue
    }
}
