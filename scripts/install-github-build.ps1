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
