[CmdletBinding()]
param(
    [long]$RunId,
    [string]$Branch = 'master',
    [string]$Repo = 'nonlog/Conduit',
    [string]$AdbSerial,
    [string]$DownloadProxy,
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

function Get-ScoopPersistDataDir([string]$InstallDir) {
    $full = [IO.Path]::GetFullPath($InstallDir).TrimEnd('\')
    if ($full -notmatch '^(?<root>.+)[\\/]apps[\\/]conduit[\\/][^\\/]+$') {
        throw "Unexpected Scoop Conduit install path: $InstallDir"
    }
    return Join-Path $Matches.root 'persist\conduit\data'
}

function Get-OptionalFileHash([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $null }
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
}

function Get-JunctionTarget([IO.DirectoryInfo]$Item) {
    if (($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0) { return $null }
    $resolved = $Item.ResolveLinkTarget($true)
    if ($null -eq $resolved) { return $null }
    return [IO.Path]::GetFullPath($resolved.FullName).TrimEnd('\')
}

function Repair-ScoopDataJunction([string]$InstallDir) {
    $dataPath = Join-Path $InstallDir 'data'
    $persistPath = Get-ScoopPersistDataDir $InstallDir
    New-Item -ItemType Directory -Force -Path $persistPath | Out-Null

    if (Test-Path -LiteralPath $dataPath) {
        $item = Get-Item -LiteralPath $dataPath -Force
        $target = Get-JunctionTarget $item
        if ($null -ne $target) {
            $expected = [IO.Path]::GetFullPath($persistPath).TrimEnd('\')
            if (-not $target.Equals($expected, [StringComparison]::OrdinalIgnoreCase)) {
                throw "Conduit data junction points to an unexpected target: $target"
            }
        } else {
            # A previous bad overlay may have replaced the Scoop junction with a fresh plain
            # directory. Persist is authoritative. If it is empty, salvage the plain directory;
            # otherwise quarantine it for this run and restore the real junction.
            if (@(Get-ChildItem -LiteralPath $persistPath -Force -ErrorAction SilentlyContinue).Count -eq 0) {
                Copy-Item -Path (Join-Path $dataPath '*') -Destination $persistPath -Recurse -Force -ErrorAction SilentlyContinue
            }
            $orphan = Join-Path $temp 'orphaned-current-data'
            if (Test-Path -LiteralPath $orphan) { Remove-Item -LiteralPath $orphan -Recurse -Force }
            Move-Item -LiteralPath $dataPath -Destination $orphan
            New-Item -ItemType Junction -Path $dataPath -Target $persistPath | Out-Null
        }
    } else {
        New-Item -ItemType Junction -Path $dataPath -Target $persistPath | Out-Null
    }

    $final = Get-Item -LiteralPath $dataPath -Force
    $finalTarget = Get-JunctionTarget $final
    $expectedTarget = [IO.Path]::GetFullPath($persistPath).TrimEnd('\')
    if ($null -eq $finalTarget -or -not $finalTarget.Equals($expectedTarget, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Conduit data persistence repair failed: $dataPath -> $finalTarget"
    }

    return [pscustomobject]@{
        DataPath = $dataPath
        PersistPath = $persistPath
        IdentityHash = Get-OptionalFileHash (Join-Path $persistPath 'identity.bin')
        ConfigHash = Get-OptionalFileHash (Join-Path $persistPath 'config.txt')
    }
}

function Download-ActionsArtifact {
    param(
        [Parameter(Mandatory)][string]$ArtifactName,
        [Parameter(Mandatory)][string]$Destination
    )

    $artifactJson = gh api "repos/$Repo/actions/runs/$RunId/artifacts"
    if ($LASTEXITCODE -ne 0) { throw "Could not query Actions artifacts for run $RunId" }
    $artifact = ($artifactJson | ConvertFrom-Json).artifacts | Where-Object name -EQ $ArtifactName | Select-Object -First 1
    if (-not $artifact) { throw "Actions artifact not found: $ArtifactName" }

    New-Item -ItemType Directory -Force -Path $Destination | Out-Null
    $archive = Join-Path $temp "$ArtifactName.zip"
    $token = (gh auth token).Trim()
    if ([string]::IsNullOrWhiteSpace($token)) { throw 'GitHub CLI has no authentication token' }

    $handler = [Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $true
    $handler.CheckCertificateRevocationList = $false
    if (-not [string]::IsNullOrWhiteSpace($DownloadProxy)) {
        $handler.UseProxy = $true
        $handler.Proxy = [Net.WebProxy]::new($DownloadProxy)
    }

    $client = [Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromMinutes(30)
    $client.DefaultRequestHeaders.UserAgent.ParseAdd('Conduit-Log-Installer/1.0')
    $client.DefaultRequestHeaders.Authorization = [Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', $token)
    try {
        $response = $client.GetAsync([string]$artifact.archive_download_url, [Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
        $response.EnsureSuccessStatusCode() | Out-Null
        $input = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
        try {
            $output = [IO.File]::Open($archive, [IO.FileMode]::Create, [IO.FileAccess]::Write, [IO.FileShare]::None)
            try { $input.CopyTo($output) } finally { $output.Dispose() }
        } finally { $input.Dispose() }
    }
    finally {
        $client.Dispose()
        $handler.Dispose()
        $token = $null
    }

    Expand-Archive -LiteralPath $archive -DestinationPath $Destination -Force
    Remove-Item -LiteralPath $archive -Force
}

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
        Download-ActionsArtifact -ArtifactName 'conduit-windows-x64' -Destination $windowsDownload

        $zip = Get-ChildItem -LiteralPath $windowsDownload -File -Filter '*.zip' | Select-Object -First 1
        if (-not $zip) { throw 'Windows Actions artifact did not contain a zip package' }
        Expand-Archive -LiteralPath $zip.FullName -DestinationPath $windowsStage -Force

        $installDir = (& scoop prefix conduit).Trim()
        if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($installDir) -or -not (Test-Path -LiteralPath $installDir -PathType Container)) {
            throw 'Conduit is not installed through Scoop on this machine'
        }

        # Development artifacts are installed on Log only. Stop the live programs, repair/verify
        # Scoop persistence first, then run the installer from the staged GitHub package. Do not
        # recursively overlay the package root: that can replace `current\data` if a build ever
        # contains a data directory.
        Get-Process Conduit, conduit-daemon -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 500
        $persistState = Repair-ScoopDataJunction $installDir

        $installer = Join-Path $windowsStage 'tools\install-windows.ps1'
        if (-not (Test-Path -LiteralPath $installer -PathType Leaf)) {
            throw "GitHub package is missing the installer: $installer"
        }
        & $installer -SourceDir $windowsStage -InstallDir $installDir

        # Keep the installed helper scripts current without touching runtime state.
        $stagedTools = Join-Path $windowsStage 'tools'
        if (Test-Path -LiteralPath $stagedTools -PathType Container) {
            Copy-Item -LiteralPath $stagedTools -Destination $installDir -Recurse -Force
        }

        $verifiedState = Repair-ScoopDataJunction $installDir
        if ($null -ne $persistState.IdentityHash -and
            $persistState.IdentityHash -ne (Get-OptionalFileHash (Join-Path $verifiedState.PersistPath 'identity.bin'))) {
            throw 'Conduit persisted identity changed during GitHub build installation'
        }
        if ($null -ne $persistState.ConfigHash -and
            $persistState.ConfigHash -ne (Get-OptionalFileHash (Join-Path $verifiedState.PersistPath 'config.txt'))) {
            throw 'Conduit persisted config changed during GitHub build installation'
        }

        # install-windows.ps1 starts the daemon normally. For this Log automation helper, detach the
        # development daemon from the invoking terminal/job as well, so AgentDock/CI shells cannot
        # reap it when their command process exits.
        Get-Process conduit-daemon -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 250
        $daemonExe = Join-Path $installDir 'conduit-daemon.exe'
        $created = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{
            CommandLine = '"' + $daemonExe + '"'
            CurrentDirectory = $installDir
        }
        if ($created.ReturnValue -ne 0) {
            throw "Could not detach the installed daemon through Win32_Process.Create (code $($created.ReturnValue))"
        }
    }

    if (-not $SkipAndroid) {
        $androidDownload = Join-Path $temp 'android-download'
        Download-ActionsArtifact -ArtifactName 'conduit-android-debug' -Destination $androidDownload

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
