[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

function Remove-ConduitShareTargetPackage {
    $exe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { return }
    $script = @"
`$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\Modules\Appx\Appx.psd1') -ErrorAction Stop
Get-AppxPackage -Name 'Conduit.Desktop.ShareTarget' -ErrorAction SilentlyContinue | Remove-AppxPackage -ErrorAction Stop
"@
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($script))
    & $exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand $encoded | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Could not remove the Conduit share-target identity package' }
}
$appData = [Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)
$programs = Join-Path $appData 'Microsoft\Windows\Start Menu\Programs'
$shortcut = Join-Path $programs 'Conduit.lnk'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$aumidKey = 'HKCU:\Software\Classes\AppUserModelId\Conduit.Desktop'
$notificationKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\Conduit.Desktop'
$explorerKey = 'HKCU:\Software\Classes\*\shell\Conduit.SendToPhone'

Get-Process conduit-daemon -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Get-Process Conduit -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Remove-ConduitShareTargetPackage
Remove-Item $shortcut -Force -ErrorAction SilentlyContinue
Remove-ItemProperty -Path $runKey -Name Conduit -ErrorAction SilentlyContinue
Remove-Item $aumidKey -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $notificationKey -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $explorerKey -Recurse -Force -ErrorAction SilentlyContinue

# User data under %LOCALAPPDATA%\Conduit is deliberately retained so pairing identity, clipboard
# history and relay preferences survive reinstall/update.
