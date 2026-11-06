[CmdletBinding()]
param([Parameter(Mandatory)][string]$PackageZip)

$ErrorActionPreference = 'Stop'
# This process-level regression owns one disposable runner's control endpoint.
# Never run it on the user's computer or against an installed device identity.
if ($env:GITHUB_ACTIONS -ne 'true' -or [string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
    throw 'Daemon exit acceptance is restricted to disposable GitHub Actions runners'
}
if (Get-Process conduit-daemon -ErrorAction SilentlyContinue) {
    throw 'Refusing to touch an existing daemon process'
}
$archive = (Resolve-Path -LiteralPath $PackageZip).Path
$root = Join-Path $env:RUNNER_TEMP ('conduit-exit-qa-' + [guid]::NewGuid().ToString('N'))
$runtime = Join-Path $root 'runtime'
$data = Join-Path $root 'private-fixture'
New-Item -ItemType Directory -Path $runtime, $data -Force | Out-Null

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class ConduitExitSmoke {
    public delegate bool EnumProc(IntPtr hwnd, IntPtr parameter);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr parameter);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr hwnd, StringBuilder name, int count);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint process);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool PostMessage(IntPtr hwnd, uint message, UIntPtr wparam, IntPtr lparam);
    public static IntPtr FindTray(int pid) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((hwnd, unused) => {
            uint owner; GetWindowThreadProcessId(hwnd, out owner);
            if (owner != (uint)pid) return true;
            var name = new StringBuilder(256); GetClassName(hwnd, name, name.Capacity);
            if (name.ToString() != "ConduitTrayWindow") return true;
            found = hwnd; return false;
        }, IntPtr.Zero);
        return found;
    }
}
'@

$process = $null
$stdout = $null
$stderr = $null
function Invoke-Control([string[]]$Arguments) {
    $start = [Diagnostics.ProcessStartInfo]::new((Join-Path $runtime 'conduit-daemon.exe'))
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.Environment['CONDUIT_DATA_DIR'] = $data
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    $client = [Diagnostics.Process]::Start($start)
    $out = $client.StandardOutput.ReadToEndAsync()
    $err = $client.StandardError.ReadToEndAsync()
    try {
        if (-not $client.WaitForExit(10000)) { $client.Kill($true); throw 'Control command timed out' }
        if ($client.ExitCode -ne 0) { throw "Control command failed: $($err.Result)" }
        return $out.Result.Trim()
    } finally { $client.Dispose() }
}

try {
    Expand-Archive -LiteralPath $archive -DestinationPath $runtime -Force
    # Explicitly paused with no relay endpoints: this test never discovers or
    # contacts a real phone, and only the local control listener is active.
    [IO.File]::WriteAllText((Join-Path $data 'config.txt'), "tray_icon=true`nrelays=`nnotification_retention_days=7`n")
    [IO.File]::WriteAllText((Join-Path $data 'link-intent.txt'), "paused`n")
    $now = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $expired = $now - [long][TimeSpan]::FromDays(8).TotalMilliseconds
    [IO.File]::WriteAllText((Join-Path $data 'notifications.tsv'),
        "$expired`told`ttest.package`tTest app`tExpired record`tRemove only this row`n$now`trecent`ttest.package`tTest app`tRecent record`tPreserve this row`n")
    $identityHash = $null
    foreach ($attempt in 1..2) {
        $start = [Diagnostics.ProcessStartInfo]::new((Join-Path $runtime 'conduit-daemon.exe'))
        $start.WorkingDirectory = $runtime
        $start.UseShellExecute = $false
        $start.CreateNoWindow = $true
        $start.RedirectStandardOutput = $true
        $start.RedirectStandardError = $true
        $start.Environment['CONDUIT_DATA_DIR'] = $data
        $process = [Diagnostics.Process]::Start($start)
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        $limit = [DateTime]::UtcNow.AddSeconds(15)
        $tray = [IntPtr]::Zero
        do {
            if ($process.HasExited) { throw "Daemon exited during startup: $($stderr.Result)" }
            $tray = [ConduitExitSmoke]::FindTray($process.Id)
            if ($tray -ne [IntPtr]::Zero -and (Test-Path (Join-Path $data 'status.txt'))) { break }
            Start-Sleep -Milliseconds 100
        } while ([DateTime]::UtcNow -lt $limit)
        if ($tray -eq [IntPtr]::Zero) { throw 'Actual native tray window did not start' }
        $currentHash = (Get-FileHash (Join-Path $data 'identity.bin') -Algorithm SHA256).Hash
        if ($identityHash -and $identityHash -ne $currentHash) { throw 'Restart changed the device identity' }
        $identityHash = $currentHash
        if ($attempt -eq 1) {
            # The actual daemon owns pruning and clearing; the UI must never race
            # a notification arrival by truncating this file itself.
            [void](Invoke-Control @('notifications','prune'))
            $history = @(Get-Content -LiteralPath (Join-Path $data 'notifications.tsv'))
            if ($history.Count -ne 1 -or $history[0] -notmatch '\trecent\t') {
                throw 'Age retention did not remove only expired notification records'
            }
            $effective = Get-Content -LiteralPath (Join-Path $data 'effective-settings.txt') -Raw
            if ($effective -notmatch '(?m)^relays_source=config$' -or $effective -notmatch '(?m)^relays=$') {
                throw 'Effective settings did not preserve explicitly disabled Relay'
            }
            [void](Invoke-Control @('notifications','clear'))
            if ((Get-Item -LiteralPath (Join-Path $data 'notifications.tsv')).Length -ne 0) {
                throw 'Serialized history clear did not clear the records'
            }
        }
        # This is the same WM_COMMAND/MENU_EXIT route as clicking Exit in the
        # tray's context menu, not a process kill or a mock channel assertion.
        if (-not [ConduitExitSmoke]::PostMessage($tray, 0x111, [UIntPtr]1002, [IntPtr]::Zero)) {
            throw 'Could not deliver the tray Exit menu action'
        }
        if (-not $process.WaitForExit(8000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw "Tray Exit left the daemon alive. Diagnostics: $($stderr.Result)"
        }
        if ($process.ExitCode -ne 0) { throw "Tray Exit failed: $($stderr.Result)" }
        if ([ConduitExitSmoke]::FindTray($process.Id) -ne [IntPtr]::Zero) { throw 'Tray window remained after exit' }
        $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Any, 41112)
        try { $listener.Start() } finally { $listener.Stop() }
        $process.Dispose()
        $process = $null
    }
    Write-Output 'DAEMON_TRAY_EXIT_PASSED: real menu exit twice, bounded shutdown, port released, identity preserved, retention/prune/clear verified'
} finally {
    if ($process) {
        if (-not $process.HasExited) { $process.Kill($true); $process.WaitForExit() }
        $process.Dispose()
    }
    $resolved = [IO.Path]::GetFullPath($root)
    $parent = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($parent, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe fixture cleanup path' }
    if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
}
