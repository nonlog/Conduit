[CmdletBinding()]
param(
    [ValidateRange(0.01, 10080.0)]
    [double]$DurationMinutes = 2880,

    [ValidateRange(1, 3600)]
    [int]$IntervalSeconds = 60,

    [ValidateRange(1, 600)]
    [int]$QuiesceSeconds = 30,

    [string]$AdbSerial,

    [string]$DaemonPath = (Join-Path $PSScriptRoot '..\target\debug\conduit-daemon.exe'),

    [switch]$Attach,

    [string]$DaemonLogPath,

    [switch]$QuiescentBaseline,

    [switch]$RestoreLink,

    [string]$OutputDir
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Package = 'com.conduit.sync'
$Service = 'com.conduit.sync/.SyncService'
$DisconnectAction = 'com.conduit.sync.DISCONNECT'
$ConnectAction = 'com.conduit.sync.CONNECT'

function Resolve-Adb {
    $cmd = Get-Command adb -ErrorAction Stop
    return $cmd.Source
}

function Resolve-Serial([string]$Requested, [string]$Adb) {
    if ($Requested) { return $Requested }

    $devices = @(
        & $Adb devices |
            Select-Object -Skip 1 |
            ForEach-Object {
                if ($_ -match '^(\S+)\s+device(?:\s|$)') { $Matches[1] }
            }
    )
    if ($devices.Count -ne 1) {
        throw "Expected exactly one ADB device, found $($devices.Count). Pass -AdbSerial explicitly."
    }
    return $devices[0]
}

function Invoke-Adb([string[]]$AdbArgs, [switch]$AllowFailure) {
    $old = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $output = & $script:Adb -s $script:Serial @AdbArgs 2>$null
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $old
    }
    if ($code -ne 0 -and !$AllowFailure) {
        throw "adb exited with ${code}: $($AdbArgs -join ' ')"
    }
    return @($output)
}

function Invoke-ServiceAction([string]$Action) {
    # SyncService is intentionally not exported. The test phone is rooted, so diagnostic
    # automation starts it as uid 0 rather than weakening the manifest just to make a soak
    # script convenient. Passing the whole `am` command as one `su -c` argument also avoids
    # shell quoting depending on whichever PowerShell happens to launch this file.
    $command = "am start-foreground-service --user 0 -n $Service -a $Action"
    $old = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        # `adb shell` reconstructs its remote command line. Passing `su`, `-c`, and the
        # command as separate native arguments loses the grouping on this Windows host;
        # send one quoted remote-shell string instead (the same form used during device
        # verification) so `su -c` receives exactly one command.
        & $script:Adb -s $script:Serial shell "su -c '$command'" | Out-Null
        $code = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $old
    }
    if ($code -ne 0) {
        throw "adb root service action exited with ${code}: $Action"
    }
}

function Get-AndroidSample {
    $pidText = (Invoke-Adb @('shell', 'pidof', $Package) -AllowFailure | Select-Object -First 1)
    if (!$pidText) {
        return [pscustomobject]@{
            AndroidPid = $null
            AndroidThreads = $null
            AndroidFds = $null
            AndroidRssKb = $null
        }
    }

    $androidPid = (($pidText -split '\s+')[0]).Trim()
    $status = (Invoke-Adb @('shell', 'cat', "/proc/$androidPid/status") -AllowFailure) -join "`n"
    $threads = if ($status -match '(?m)^Threads:\s+(\d+)') { [int]$Matches[1] } else { $null }
    $rssKb = if ($status -match '(?m)^VmRSS:\s+(\d+)\s+kB') { [int64]$Matches[1] } else { $null }

    # /proc/<other uid>/fd is hidden from the shell on the target device. Root is used only
    # for this count; failure leaves the field blank rather than aborting a 48-hour run.
    $fdLine = Invoke-Adb @('shell', "su -c 'ls /proc/$androidPid/fd | wc -l'") -AllowFailure |
        Select-Object -First 1
    $fds = if ($fdLine -and $fdLine.Trim() -match '^\d+$') { [int]$fdLine.Trim() } else { $null }

    return [pscustomobject]@{
        AndroidPid = [int]$androidPid
        AndroidThreads = $threads
        AndroidFds = $fds
        AndroidRssKb = $rssKb
    }
}

function Get-WindowsSample([int]$DaemonPid) {
    $process = Get-Process -Id $DaemonPid -ErrorAction Stop
    $tcp = @(Get-NetTCPConnection -OwningProcess $DaemonPid -ErrorAction SilentlyContinue)
    return [pscustomobject]@{
        WindowsPid = $DaemonPid
        WindowsThreads = $process.Threads.Count
        WindowsHandles = $process.HandleCount
        WindowsWorkingSetKb = [math]::Round($process.WorkingSet64 / 1KB)
        WindowsPrivateKb = [math]::Round($process.PrivateMemorySize64 / 1KB)
        WindowsTcpEstablished = @($tcp | Where-Object State -eq 'Established').Count
        WindowsTcpListen = @($tcp | Where-Object State -eq 'Listen').Count
        WindowsTcpTotal = $tcp.Count
    }
}

function Add-Sample([string]$Phase, [int]$DaemonPid, [string]$Csv) {
    $win = Get-WindowsSample $DaemonPid
    $android = Get-AndroidSample
    $sample = [pscustomobject]@{
        TimestampUtc = [DateTime]::UtcNow.ToString('o')
        Phase = $Phase
        WindowsPid = $win.WindowsPid
        WindowsThreads = $win.WindowsThreads
        WindowsHandles = $win.WindowsHandles
        WindowsWorkingSetKb = $win.WindowsWorkingSetKb
        WindowsPrivateKb = $win.WindowsPrivateKb
        WindowsTcpEstablished = $win.WindowsTcpEstablished
        WindowsTcpListen = $win.WindowsTcpListen
        WindowsTcpTotal = $win.WindowsTcpTotal
        AndroidPid = $android.AndroidPid
        AndroidThreads = $android.AndroidThreads
        AndroidFds = $android.AndroidFds
        AndroidRssKb = $android.AndroidRssKb
    }
    $sample | Export-Csv -LiteralPath $Csv -NoTypeInformation -Append -Encoding utf8
    Write-Host (
        '[soak] {0} {1} win(t={2} h={3} ws={4}K tcp={5}) android(pid={6} t={7} fd={8} rss={9}K)' -f
        $sample.TimestampUtc, $Phase, $sample.WindowsThreads, $sample.WindowsHandles,
        $sample.WindowsWorkingSetKb, $sample.WindowsTcpEstablished, $sample.AndroidPid,
        $sample.AndroidThreads, $sample.AndroidFds, $sample.AndroidRssKb
    )
    return $sample
}

function Get-LastLifecycle([string]$Path, [string]$Pattern, [string[]]$Names) {
    if (!$Path -or !(Test-Path -LiteralPath $Path)) { return $null }
    $lines = Get-Content -LiteralPath $Path -ErrorAction SilentlyContinue
    return Get-LastLifecycleFromLines $lines $Pattern $Names
}

function Get-LastLifecycleFromLines([string[]]$Lines, [string]$Pattern, [string[]]$Names) {
    if (!$Lines) { return $null }
    $match = $Lines | Select-String -Pattern $Pattern -AllMatches -ErrorAction SilentlyContinue |
        Select-Object -Last 1
    if (!$match) { return $null }
    $m = [regex]::Match($match.Line, $Pattern)
    if (!$m.Success) { return $null }
    $result = [ordered]@{}
    for ($i = 0; $i -lt $Names.Count; $i++) {
        $result[$Names[$i]] = [int64]$m.Groups[$i + 1].Value
    }
    return [pscustomobject]$result
}

function Get-AndroidLifecycleSnapshot {
    $lines = Invoke-Adb @(
        'logcat', '-d', '-b', 'all', '-v', 'threadtime', '-s',
        'conduit.link:I', 'conduit.svc:I', '*:S'
    ) -AllowFailure
    return Get-LastLifecycleFromLines $lines 'opened=(\d+).*closed=(\d+)' @('Opened', 'Closed')
}

function Delta($First, $Last, [string]$Name) {
    if ($null -eq $First -or $null -eq $Last) { return $null }
    $a = $First.$Name
    $b = $Last.$Name
    if ($null -eq $a -or $null -eq $b -or $a -eq '' -or $b -eq '') { return $null }
    return [int64]$b - [int64]$a
}

$script:Adb = Resolve-Adb
$script:Serial = Resolve-Serial $AdbSerial $script:Adb

if (!$OutputDir) {
    $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
    $OutputDir = Join-Path $env:TEMP "conduit-soak-$stamp"
}
$OutputDir = [IO.Path]::GetFullPath($OutputDir)
New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null

$samplesPath = Join-Path $OutputDir 'samples.csv'
$androidLog = Join-Path $OutputDir 'android.log'
$androidErr = Join-Path $OutputDir 'android.err.log'
$summaryPath = Join-Path $OutputDir 'summary.json'
$launched = $false
$daemon = $null
$logcat = $null
$logcatExitedEarly = $false

if ($Attach) {
    $existing = @(Get-Process conduit-daemon -ErrorAction SilentlyContinue)
    if ($existing.Count -ne 1) {
        throw "-Attach requires exactly one running conduit-daemon process; found $($existing.Count)."
    }
    $daemon = $existing[0]
    if (!$DaemonLogPath) {
        Write-Warning '-Attach without -DaemonLogPath cannot prove Windows created/closed lifecycle counters.'
    } elseif (!(Test-Path -LiteralPath $DaemonLogPath)) {
        throw "Daemon log not found: $DaemonLogPath"
    }
} else {
    $listener = @(Get-NetTCPConnection -LocalPort 41112 -State Listen -ErrorAction SilentlyContinue)
    if ($listener.Count -gt 0) {
        throw 'TCP 41112 already has a listener. Stop the existing Conduit daemon or use -Attach.'
    }
    $DaemonPath = [IO.Path]::GetFullPath($DaemonPath)
    if (!(Test-Path -LiteralPath $DaemonPath)) { throw "Daemon not found: $DaemonPath" }
    $DaemonLogPath = Join-Path $OutputDir 'daemon.log'
    $daemonErr = Join-Path $OutputDir 'daemon.err.log'
    $daemon = Start-Process -FilePath $DaemonPath -WorkingDirectory (Split-Path $DaemonPath) `
        -WindowStyle Hidden -RedirectStandardOutput $DaemonLogPath -RedirectStandardError $daemonErr `
        -PassThru
    $launched = $true
    Start-Sleep -Milliseconds 750
    if ($daemon.HasExited) { throw "Conduit daemon exited during startup. See $daemonErr" }
}

$initialAndroidLifecycle = Get-AndroidLifecycleSnapshot
$logcatArgs = @(
    '-s', $script:Serial, 'logcat', '-b', 'all', '-v', 'threadtime', '-s',
    'conduit.link:I', 'conduit.svc:I', '*:S'
)
$logcat = Start-Process -FilePath $script:Adb -ArgumentList $logcatArgs -WindowStyle Hidden `
    -RedirectStandardOutput $androidLog -RedirectStandardError $androidErr -PassThru

Write-Host "Conduit soak output: $OutputDir"
Write-Host "ADB serial: $script:Serial"
Write-Host "Daemon PID: $($daemon.Id)"
Write-Host "Duration: $DurationMinutes min; interval: $IntervalSeconds s"
if ($QuiescentBaseline) { Write-Host "Quiescent settle: $QuiesceSeconds s" }

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$gitHead = (& git -C $repoRoot rev-parse HEAD 2>$null | Select-Object -First 1)
$daemonSha256 = if (Test-Path -LiteralPath $DaemonPath) {
    (Get-FileHash -LiteralPath $DaemonPath -Algorithm SHA256).Hash.ToLowerInvariant()
} else { $null }

$initial = $null
$last = $null
$quiescent = $null
$deadline = $null
$runError = $null

try {
    if ($QuiescentBaseline) {
        Invoke-ServiceAction $DisconnectAction
        Start-Sleep -Seconds $QuiesceSeconds
        $initial = Add-Sample 'baseline-quiescent' $daemon.Id $samplesPath
        Invoke-ServiceAction $ConnectAction
        Start-Sleep -Seconds 3
    } else {
        $initial = Add-Sample 'baseline' $daemon.Id $samplesPath
    }

    # Setup/settling is not part of the requested soak duration. This matters most for
    # short diagnostics, and keeps a 48-hour run exactly 48 hours of observed steady state.
    $deadline = [DateTime]::UtcNow.AddMinutes($DurationMinutes)

    while ([DateTime]::UtcNow -lt $deadline) {
        $sleep = [Math]::Min($IntervalSeconds, [Math]::Max(0.1, ($deadline - [DateTime]::UtcNow).TotalSeconds))
        Start-Sleep -Milliseconds ([int]($sleep * 1000))
        if ($daemon.HasExited) { throw "Conduit daemon exited with code $($daemon.ExitCode)." }
        if ($logcat.HasExited) { $logcatExitedEarly = $true }
        $last = Add-Sample 'run' $daemon.Id $samplesPath
    }

    if ($QuiescentBaseline) {
        Invoke-ServiceAction $DisconnectAction
        Start-Sleep -Seconds $QuiesceSeconds
        $quiescent = Add-Sample 'final-quiescent' $daemon.Id $samplesPath
        $last = $quiescent
    }
} catch {
    $runError = $_.Exception.Message
    Write-Error $runError
} finally {
    if ($logcat -and !$logcat.HasExited) {
        Stop-Process -Id $logcat.Id -Force -ErrorAction SilentlyContinue
        $logcat.WaitForExit(5000) | Out-Null
    } elseif ($logcat) {
        $logcatExitedEarly = $true
    }
}

$windowsLifecycle = Get-LastLifecycle $DaemonLogPath `
    'created=(\d+).*closed=(\d+)' @('Created', 'Closed')
$capturedAndroidLifecycle = Get-LastLifecycle $androidLog `
    'opened=(\d+).*closed=(\d+)' @('Opened', 'Closed')
$androidLifecycle = if ($capturedAndroidLifecycle) {
    $capturedAndroidLifecycle
} else {
    $initialAndroidLifecycle
}
$recorded = if (Test-Path -LiteralPath $samplesPath) { @(Import-Csv -LiteralPath $samplesPath) } else { @() }
$windowsPids = @($recorded.WindowsPid | Where-Object { $_ } | Sort-Object -Unique)
$androidPids = @($recorded.AndroidPid | Where-Object { $_ } | Sort-Object -Unique)
$androidSamples = @($recorded | Where-Object { $_.AndroidPid }).Count
$windowsGap = if ($windowsLifecycle) { $windowsLifecycle.Created - $windowsLifecycle.Closed } else { $null }
$androidGap = if ($androidLifecycle) { $androidLifecycle.Opened - $androidLifecycle.Closed } else { $null }

$summary = [ordered]@{
    StartedUtc = if ($initial) { $initial.TimestampUtc } else { $null }
    FinishedUtc = [DateTime]::UtcNow.ToString('o')
    DurationMinutes = $DurationMinutes
    IntervalSeconds = $IntervalSeconds
    QuiesceSeconds = if ($QuiescentBaseline) { $QuiesceSeconds } else { $null }
    OutputDir = $OutputDir
    AdbSerial = $script:Serial
    GitHead = $gitHead
    DaemonSha256 = $daemonSha256
    DaemonPid = if ($daemon) { $daemon.Id } else { $null }
    DaemonLaunchedByScript = $launched
    SampleCount = $recorded.Count
    AndroidSampleCount = $androidSamples
    AndroidCoveragePercent = if ($recorded.Count -gt 0) {
        [math]::Round(100.0 * $androidSamples / $recorded.Count, 2)
    } else { $null }
    WindowsPidCount = $windowsPids.Count
    AndroidPidCount = $androidPids.Count
    Error = $runError
    LogcatExitedEarly = $logcatExitedEarly
    WindowsLifecycle = $windowsLifecycle
    InitialAndroidLifecycle = $initialAndroidLifecycle
    AndroidLifecycle = $androidLifecycle
    AndroidLifecycleSource = if ($capturedAndroidLifecycle) {
        'captured-run'
    } elseif ($initialAndroidLifecycle) {
        'pre-run-snapshot'
    } else { $null }
    Deltas = if ($initial -and $last) {
        [ordered]@{
            WindowsThreads = Delta $initial $last 'WindowsThreads'
            WindowsHandles = Delta $initial $last 'WindowsHandles'
            WindowsWorkingSetKb = Delta $initial $last 'WindowsWorkingSetKb'
            WindowsPrivateKb = Delta $initial $last 'WindowsPrivateKb'
            WindowsTcpTotal = Delta $initial $last 'WindowsTcpTotal'
            AndroidThreads = Delta $initial $last 'AndroidThreads'
            AndroidFds = Delta $initial $last 'AndroidFds'
            AndroidRssKb = Delta $initial $last 'AndroidRssKb'
        }
    } else { $null }
    Invariants = [ordered]@{
        WindowsLifecycleGap = $windowsGap
        AndroidLifecycleGap = $androidGap
        WindowsLifecycleWithinOne = if ($null -ne $windowsGap) {
            $windowsGap -ge 0 -and $windowsGap -le 1
        } else { $null }
        AndroidLifecycleWithinOne = if ($null -ne $androidGap) {
            $androidGap -ge 0 -and $androidGap -le 1
        } else { $null }
        WindowsQuiescentEqualsClosed = if ($QuiescentBaseline -and $null -ne $windowsGap) {
            $windowsGap -eq 0
        } else { $null }
        AndroidQuiescentEqualsClosed = if ($QuiescentBaseline -and $null -ne $androidGap) {
            $androidGap -eq 0
        } else { $null }
        WindowsPidStable = $windowsPids.Count -eq 1
        AndroidPidStable = if ($androidPids.Count -gt 0) { $androidPids.Count -eq 1 } else { $null }
        WindowsThreadNoGrowth = if ($initial -and $last) {
            (Delta $initial $last 'WindowsThreads') -le 0
        } else { $null }
        WindowsHandleNoGrowth = if ($initial -and $last) {
            (Delta $initial $last 'WindowsHandles') -le 0
        } else { $null }
        WindowsTcpNoGrowth = if ($initial -and $last) {
            (Delta $initial $last 'WindowsTcpTotal') -le 0
        } else { $null }
        AndroidThreadNoGrowth = if ($initial -and $last -and $null -ne (Delta $initial $last 'AndroidThreads')) {
            (Delta $initial $last 'AndroidThreads') -le 0
        } else { $null }
        AndroidFdNoGrowth = if ($initial -and $last -and $null -ne (Delta $initial $last 'AndroidFds')) {
            (Delta $initial $last 'AndroidFds') -le 0
        } else { $null }
    }
}

$summary | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $summaryPath -Encoding utf8
Write-Host "Summary: $summaryPath"
if ($QuiescentBaseline -and $RestoreLink) {
    try {
        # Restore only after logs/counters are frozen into the summary. Otherwise the new
        # active session correctly changes the lifecycle gap back to one and makes a valid
        # quiescent result look like a failure.
        Invoke-ServiceAction $ConnectAction
    } catch {
        Write-Warning "Could not restore the Conduit link: $($_.Exception.Message)"
    }
}
if ($runError) { exit 1 }
