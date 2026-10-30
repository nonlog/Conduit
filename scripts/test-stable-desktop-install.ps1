[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$PackageZip
)

$ErrorActionPreference = 'Stop'
# This script changes the current-user Start shortcut and sparse MSIX identity.
# It must run only on a disposable GitHub-hosted Windows runner, never on the
# user's installed Conduit machine or against real pairing data.
if ($env:GITHUB_ACTIONS -ne 'true' -or [string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
    throw 'Native stable install smoke is restricted to disposable GitHub Actions runners'
}
$stage = Join-Path $env:RUNNER_TEMP ('conduit-stable-install-' + [guid]::NewGuid().ToString('N'))
$install = Join-Path $stage 'install'
$zipPath = (Resolve-Path -LiteralPath $PackageZip).Path
$ps5 = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
function Get-ShareVersion {
    $script = '$package = Get-AppxPackage -Name Conduit.Desktop.ShareTarget -ErrorAction SilentlyContinue; if ($package) { $package.Version.ToString() }'
    $result = & $ps5 -NoProfile -NonInteractive -Command $script 2>$null
    if ($LASTEXITCODE -ne 0) { throw 'Could not inspect the native Share Target registration' }
    return ($result | Select-Object -First 1)
}
if (Get-ShareVersion) { throw 'Runner already has a Share Target: refusing to mutate an existing installation' }
New-Item -ItemType Directory -Path $install -Force | Out-Null
$installed = $false
$installAttempted = $false
try {
    Expand-Archive -LiteralPath $zipPath -DestinationPath $install -Force
    $state = Join-Path $install 'data'
    New-Item -ItemType Directory -Path $state -Force | Out-Null
    $identity = Join-Path $state 'identity.bin'
    [IO.File]::WriteAllBytes($identity, [byte[]](9, 8, 7, 6, 5, 4, 3, 2))
    $before = (Get-FileHash -LiteralPath $identity -Algorithm SHA256).Hash

    $installAttempted = $true
    $results = @(& (Join-Path $install 'tools/install-windows.ps1') -SourceDir $install -InstallDir $install -NoStart)
    $result = $results | Where-Object { $_ -is [pscustomobject] } | Select-Object -Last 1
    $installed = $true
    if (-not $result -or $result.DefaultDesktop -ne 'Rune' -or
        -not $result.ShareTargetRegistered -or $result.DaemonStarted) {
        throw "Stable install result did not select Rune while retaining Share Target: $($result | Out-String)"
    }
    if ((Get-FileHash -LiteralPath $identity -Algorithm SHA256).Hash -ne $before) {
        throw 'Stable installer modified persisted pairing identity'
    }

    $shortcutPath = Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'Microsoft\Windows\Start Menu\Programs\Conduit.lnk'
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($shortcutPath)
    if (-not $shortcut.TargetPath.EndsWith('conduit-rune.exe', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Stable shortcut did not select Rune: $($shortcut.TargetPath)"
    }
    $shareVersion = Get-ShareVersion
    if ($shareVersion -ne '0.1.13.0') {
        throw "Share Target sparse MSIX registration not upgraded: $shareVersion"
    }
    Write-Output 'STABLE_INSTALL_SMOKE_PASSED: Rune shortcut, Nova fallback, Share Target registration, identity preservation, daemon not started'
}
finally {
    try {
        # Even a partially failed install can have registered the sparse MSIX
        # and Start menu shortcut. Cleanup the disposable runner on failure.
        if ($installAttempted) {
            & (Join-Path $install 'tools/uninstall-windows.ps1')
            if (Get-ShareVersion) { throw 'Share Target registration remained after smoke uninstall' }
        }
    }
    finally {
        if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
    }
}
