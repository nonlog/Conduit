[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$appData = [Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)
$programs = Join-Path $appData 'Microsoft\Windows\Start Menu\Programs'
$shortcut = Join-Path $programs 'Conduit.lnk'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$aumidKey = 'HKCU:\Software\Classes\AppUserModelId\Conduit.Desktop'
$notificationKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings\Conduit.Desktop'
$explorerKey = 'HKCU:\Software\Classes\*\shell\Conduit.SendToPhone'

Get-Process conduit-daemon -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Get-Process Conduit -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Remove-Item $shortcut -Force -ErrorAction SilentlyContinue
Remove-ItemProperty -Path $runKey -Name Conduit -ErrorAction SilentlyContinue
Remove-Item $aumidKey -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $notificationKey -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item $explorerKey -Recurse -Force -ErrorAction SilentlyContinue

# User data under %LOCALAPPDATA%\Conduit is deliberately retained so pairing identity, clipboard
# history and relay preferences survive reinstall/update.
