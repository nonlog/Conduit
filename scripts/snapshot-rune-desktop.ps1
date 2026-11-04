[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string]$Executable,
    [Parameter(Mandatory)] [string]$OutputDir
)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true' -or [string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
    throw 'Formal native visual acceptance runs on GitHub Actions only'
}
$exe = (Resolve-Path -LiteralPath $Executable).Path
$output = [IO.Path]::GetFullPath($OutputDir)
New-Item -ItemType Directory -Path $output -Force | Out-Null
$fixture = Join-Path $env:RUNNER_TEMP ('conduit-rune-visual-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixture | Out-Null
$previous = $env:CONDUIT_DATA_DIR
try {
    # Headless scenes contain synthetic data. They never start a daemon, read
    # production history, write an identity, or open a desktop window.
    $env:CONDUIT_DATA_DIR = $fixture
    foreach ($scene in @('qa-overview','qa-transfers','qa-notifications','qa-links','qa-pairing','qa-settings','qa-offline')) {
        foreach ($theme in @('dark','light')) {
            foreach ($size in @('760x510','1020x605')) {
                $png = Join-Path $output "$scene-$theme-$size.png"
                # A PE GUI executable is asynchronous when invoked with '&' in PowerShell.
                # Wait on this exact renderer process instead of reading a stale LASTEXITCODE.
                $start = [Diagnostics.ProcessStartInfo]::new($exe)
                $start.UseShellExecute = $false
                $start.CreateNoWindow = $true
                foreach ($arg in @('--snap',$png,'--scene',$scene,'--theme',$theme,'--size',$size,'--scale','1.25','--at','0.5')) {
                    $start.ArgumentList.Add($arg)
                }
                $process = [Diagnostics.Process]::Start($start)
                try {
                    if (-not $process.WaitForExit(45000)) {
                        $process.Kill($true)
                        $process.WaitForExit()
                        throw "Native rendering timed out for $scene/$theme/$size"
                    }
                    if ($process.ExitCode -ne 0) {
                        throw "Native rendering failed for $scene/$theme/$size (exit $($process.ExitCode))"
                    }
                } finally {
                    $process.Dispose()
                }
                if (-not (Test-Path -LiteralPath $png) -or (Get-Item -LiteralPath $png).Length -lt 1KB) {
                    throw "Native renderer did not produce a usable image: $png"
                }
            }
        }
    }
    if (@(Get-ChildItem -LiteralPath $output -Filter '*.png').Count -ne 28) {
        throw 'Expected 28 native visual acceptance screenshots'
    }
}
finally {
    $env:CONDUIT_DATA_DIR = $previous
    if (Test-Path -LiteralPath $fixture) { Remove-Item -LiteralPath $fixture -Recurse -Force }
}
