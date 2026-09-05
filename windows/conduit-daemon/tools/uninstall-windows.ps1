[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

function Remove-ConduitShareTargetPackage([string]$InstallDir) {
    $certificatePath = Join-Path $InstallDir 'Conduit.ShareTarget.cer'
    $thumbprint = $null
    if (Test-Path -LiteralPath $certificatePath -PathType Leaf) {
        $certificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new($certificatePath)
        try { $thumbprint = $certificate.Thumbprint } finally { $certificate.Dispose() }
    }

    $exe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    if (Test-Path -LiteralPath $exe -PathType Leaf) {
        $script = @"
`$ErrorActionPreference = 'Stop'
Import-Module (Join-Path `$env:SystemRoot 'System32\WindowsPowerShell\v1.0\Modules\Appx\Appx.psd1') -ErrorAction Stop
Get-AppxPackage -Name 'Conduit.Desktop.ShareTarget' -ErrorAction SilentlyContinue | Remove-AppxPackage -ErrorAction Stop
"@
        $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($script))
        & $exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand $encoded | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'Could not remove the Conduit share-target identity package' }
    }

    if (-not [string]::IsNullOrWhiteSpace($thumbprint)) {
        # Current builds leave no trust entry behind. These removals also clean up any residue from
        # development builds that used either CurrentUser or LocalMachine certificate stores.
        & certutil.exe -delstore TrustedPeople $thumbprint | Out-Null
        foreach ($storeName in @(
            [Security.Cryptography.X509Certificates.StoreName]::TrustedPeople,
            [Security.Cryptography.X509Certificates.StoreName]::Root)) {
            $store = [Security.Cryptography.X509Certificates.X509Store]::new(
                $storeName,
                [Security.Cryptography.X509Certificates.StoreLocation]::CurrentUser)
            try {
                $store.Open([Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite)
                @($store.Certificates.Find(
                    [Security.Cryptography.X509Certificates.X509FindType]::FindByThumbprint,
                    $thumbprint,
                    $false)) | ForEach-Object { $store.Remove($_) }
            }
            finally { $store.Dispose() }
        }
    }
}
$localAppData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
$appData = [Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)
$installDir = Join-Path $localAppData 'Programs\Conduit'
# Scoop deployments call this script from the installed tools directory; prefer that runtime root.
$scriptRuntime = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (Test-Path -LiteralPath (Join-Path $scriptRuntime 'Conduit.exe') -PathType Leaf) { $installDir = $scriptRuntime }
$programs = Join-Path $appData 'Microsoft\Windows\Start Menu\Programs'
$shortcut = Join-Path $programs 'Conduit.lnk'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$aumidKey = 'HKCU:\Software\Classes\AppUserModelId\Conduit.Desktop'
$notificationKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\Conduit.Desktop'
$explorerKey = 'HKCU:\Software\Classes\*\shell\Conduit.SendToPhone'

Get-Process conduit-daemon -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Get-Process Conduit -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Remove-ConduitShareTargetPackage $installDir
Remove-Item $shortcut -Force -ErrorAction SilentlyContinue
Remove-ItemProperty -Path $runKey -Name Conduit -ErrorAction SilentlyContinue
Remove-Item $aumidKey -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $notificationKey -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $explorerKey -Recurse -Force -ErrorAction SilentlyContinue

# User data under %LOCALAPPDATA%\Conduit is deliberately retained so pairing identity, clipboard
# history and relay preferences survive reinstall/update.
