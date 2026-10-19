[CmdletBinding(SupportsShouldProcess)]
param(
    [Parameter(Mandatory)][string]$DaemonPath,
    [switch]$CheckOnly
)

$ErrorActionPreference = 'Stop'
$daemon = Get-Item -LiteralPath $DaemonPath -ErrorAction Stop
if ($daemon.PSIsContainer -or $daemon.Name -ine 'conduit-daemon.exe') {
    throw 'The LAN firewall rules must target the installed conduit-daemon.exe.'
}
# Scoop current is a junction. Windows application firewall matching uses the actual versioned
# binary path, so refresh that path on every update rather than leaving rules for old releases.
$directory = Get-Item -LiteralPath $daemon.DirectoryName -Force
$target = @($directory.Target) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -First 1
$program = if ($target) { Join-Path $target $daemon.Name } else { $daemon.FullName }
$program = [IO.Path]::GetFullPath($program)
if (-not (Test-Path -LiteralPath $program -PathType Leaf)) { throw "Installed daemon not found: $program" }

$specs = @(
    @{ Name = 'Conduit.Managed.LAN.TCP'; Title = 'Conduit LAN (local subnet)'; Protocol = 'TCP'; Number = 6; Port = 41112 },
    @{ Name = 'Conduit.Managed.mDNS.UDP'; Title = 'Conduit discovery (local subnet)'; Protocol = 'UDP'; Number = 17; Port = 5353 }
)
$configured = $true
foreach ($spec in $specs) {
    $existing = Get-NetFirewallRule -Name $spec.Name -PolicyStore PersistentStore -ErrorAction SilentlyContinue
    if ($existing -and $existing.Group -ne 'Conduit') {
        throw "Firewall rule name belongs to another group: $($spec.Name)"
    }
    $matches = $false
    if ($existing) {
        $app = $existing | Get-NetFirewallApplicationFilter
        $port = $existing | Get-NetFirewallPortFilter
        $address = $existing | Get-NetFirewallAddressFilter
        $matches = $existing.Enabled -eq 'True' -and $existing.Direction -eq 'Inbound' -and
            $existing.Action -eq 'Allow' -and $existing.Profile -eq 'Any' -and
            [string]::Equals($app.Program, $program, [StringComparison]::OrdinalIgnoreCase) -and
            ($port.Protocol -eq $spec.Protocol -or $port.Protocol -eq [string]$spec.Number) -and
            ($port.LocalPort -join ',') -eq [string]$spec.Port -and
            ($address.RemoteAddress -join ',') -eq 'LocalSubnet'
    }
    if ($matches) { continue }
    $configured = $false
    if ($CheckOnly) { continue }
    if ($PSCmdlet.ShouldProcess($program, "Allow $($spec.Protocol):$($spec.Port) from LocalSubnet only")) {
        $parameters = @{
            Name = $spec.Name; PolicyStore = 'PersistentStore'; Direction = 'Inbound'
            Action = 'Allow'; Enabled = 'True'; Profile = 'Any'; Program = $program
            Protocol = $spec.Protocol; LocalPort = $spec.Port; RemoteAddress = 'LocalSubnet'
        }
        if ($existing) {
            Set-NetFirewallRule @parameters | Out-Null
        } else {
            New-NetFirewallRule @parameters -DisplayName $spec.Title -Group 'Conduit' | Out-Null
        }
    }
}
if ($CheckOnly) {
    if ($configured) { 'CONDUIT_LAN_FIREWALL_READY' } else { 'CONDUIT_LAN_FIREWALL_UPDATE_REQUIRED' }
} elseif (-not $WhatIfPreference) {
    # Read back instead of claiming success after a skipped or blocked operation.
    & $PSCommandPath -DaemonPath $program -CheckOnly
}
