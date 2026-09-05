[CmdletBinding()]
param(
    [string]$SourceDir = (Join-Path $PSScriptRoot '..\..\..\target\release'),
    [string]$InstallDir,
    [switch]$NoStart
)

$ErrorActionPreference = 'Stop'

function Invoke-WindowsPowerShell([string]$Script) {
    $exe = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
        throw "Windows PowerShell is required for Appx registration: $exe"
    }
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($Script))
    $output = & $exe -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand $encoded 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "Windows PowerShell command failed ($LASTEXITCODE): $($output -join [Environment]::NewLine)"
    }
    return @($output)
}

$source = (Resolve-Path $SourceDir).Path
$localAppData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
$appData = [Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)
if ([string]::IsNullOrWhiteSpace($localAppData) -or [string]::IsNullOrWhiteSpace($appData)) {
    throw 'Windows Known Folder lookup for AppData failed'
}
$installDir = if ([string]::IsNullOrWhiteSpace($InstallDir)) {
    Join-Path $localAppData 'Programs\Conduit'
} else {
    [IO.Path]::GetFullPath($InstallDir)
}
$programs = Join-Path $appData 'Microsoft\Windows\Start Menu\Programs'
$shortcut = Join-Path $programs 'Conduit.lnk'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$aumidKey = 'HKCU:\Software\Classes\AppUserModelId\Conduit.Desktop'
$explorerVerbKey = 'Registry::HKEY_CURRENT_USER\Software\Classes\*\shell\Conduit.SendToPhone'

$controlSource = @('Conduit.exe', 'conduit-control.exe') |
    ForEach-Object { Join-Path $source $_ } |
    Where-Object { Test-Path $_ } |
    Select-Object -First 1
if (-not $controlSource) { throw "Missing Conduit.exe/conduit-control.exe in $source" }
$required = @('conduit-daemon.exe', 'conduit-send.exe')
foreach ($name in $required) {
    if (-not (Test-Path (Join-Path $source $name))) {
        throw "Missing $name in $source"
    }
}

$assetDir = Join-Path $PSScriptRoot '..\assets'
$requiredIconAssets = @('conduit-icon.ico', 'conduit-icon.png', 'conduit-icon-light.ico', 'conduit-icon-light.png', 'conduit-icon-dark.ico', 'conduit-icon-dark.png')
$optionalIconAssets = @('conduit-explorer-light.ico', 'conduit-explorer-dark.ico')
foreach ($name in $requiredIconAssets) {
    if (-not (Test-Path (Join-Path $assetDir $name))) {
        throw "Missing icon asset $name"
    }
}

# Preserve the user's current Start-at-sign-in choice. Installation updates the executable path
# only when the option was already enabled; it never silently opts the user in.
$hadAutostart = $null -ne (Get-ItemProperty -Path $runKey -Name Conduit -ErrorAction SilentlyContinue).Conduit
$hadExplorerIntegration = Test-Path -LiteralPath $explorerVerbKey

$running = @(Get-Process conduit-daemon, Conduit -ErrorAction SilentlyContinue)
if ($running.Count -gt 0) {
    $running | Stop-Process -Force -ErrorAction SilentlyContinue
    # Stop-Process requests termination but the executable image can stay mapped for a short
    # moment. Wait for actual process exit before replacing binaries so an update never races the
    # final file-handle release on a busy desktop.
    $running | Wait-Process -Timeout 5 -ErrorAction SilentlyContinue
    $stuck = @($running | Where-Object { -not $_.HasExited })
    if ($stuck.Count -gt 0) {
        throw "Conduit processes did not exit before update: $($stuck.Id -join ', ')"
    }
}
New-Item -ItemType Directory -Force -Path $installDir, $programs | Out-Null

# A packaged Conduit build contains the complete self-contained WinUI publish, not just the three
# executable entry points. Updating only Conduit.exe leaves the old Conduit.dll/PRI/resources in
# place, which means XAML and code-behind changes never reach the running UI. When the source is a
# published package, copy the whole runtime payload while deliberately preserving the install's
# data junction and keeping installer tooling out of the runtime root.
$hasPublishedUi = Test-Path -LiteralPath (Join-Path $source 'Conduit.dll') -PathType Leaf
if ($hasPublishedUi) {
    foreach ($entry in Get-ChildItem -LiteralPath $source -Force) {
        if ($entry.Name -in @('tools', 'data')) { continue }
        $to = Join-Path $installDir $entry.Name
        if ([string]::Equals($entry.FullName, $to, [StringComparison]::OrdinalIgnoreCase)) { continue }
        if ($entry.PSIsContainer) {
            Copy-Item -LiteralPath $entry.FullName -Destination $installDir -Recurse -Force
        } else {
            Copy-Item -LiteralPath $entry.FullName -Destination $to -Force
        }
    }
} else {
    # Developer/repo invocation still supports a Rust-only target/release directory.
    foreach ($name in $required) {
        $from = (Resolve-Path (Join-Path $source $name)).Path
        $to = Join-Path $installDir $name
        if (-not [string]::Equals($from, $to, [StringComparison]::OrdinalIgnoreCase)) {
            Copy-Item -Force $from $to
        }
    }
    if (-not [string]::Equals((Resolve-Path $controlSource).Path, (Join-Path $installDir 'Conduit.exe'), [StringComparison]::OrdinalIgnoreCase)) {
        Copy-Item -Force $controlSource (Join-Path $installDir 'Conduit.exe')
    }
}
if ($hasPublishedUi) {
    foreach ($name in @('Conduit.dll', 'Conduit.pri')) {
        $from = Join-Path $source $name
        $to = Join-Path $installDir $name
        if (-not (Test-Path -LiteralPath $to -PathType Leaf)) {
            throw "Published UI payload did not install $name"
        }
        if ((Get-FileHash -LiteralPath $from -Algorithm SHA256).Hash -ne
            (Get-FileHash -LiteralPath $to -Algorithm SHA256).Hash) {
            throw "Published UI payload hash mismatch after installing $name"
        }
    }
}
Remove-Item (Join-Path $installDir 'conduit-control.exe') -Force -ErrorAction SilentlyContinue
foreach ($name in @($requiredIconAssets + $optionalIconAssets)) {
    $candidate = Join-Path $assetDir $name
    if (-not (Test-Path $candidate)) { continue }
    $from = (Resolve-Path (Join-Path $assetDir $name)).Path
    $to = Join-Path $installDir $name
    if (-not [string]::Equals($from, $to, [StringComparison]::OrdinalIgnoreCase)) {
        Copy-Item -Force $from $to
    }
}

# Windows Share targets require package identity even though Conduit's binaries stay managed by
# Scoop/the existing installer. On Windows 11 the tiny sparse identity package can remain unsigned
# because it contains only a manifest; the executable and all assets continue to live here at the
# external location. Re-register on update so the share contract always points at `current`.
$sharePackageName = 'Conduit.Desktop.ShareTarget'
$sharePackage = Join-Path $installDir 'Conduit.ShareTarget.msix'
$shareTargetRegistered = $false
if ((Test-Path -LiteralPath $sharePackage -PathType Leaf) -and [Environment]::OSVersion.Version.Build -ge 22000) {
    $packageLiteral = "'" + $sharePackage.Replace("'", "''") + "'"
    $locationLiteral = "'" + $installDir.Replace("'", "''") + "'"
    $registerScript = @"
`$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\Modules\Appx\Appx.psd1') -ErrorAction Stop
Get-AppxPackage -Name '$sharePackageName' -ErrorAction SilentlyContinue | Remove-AppxPackage -ErrorAction Stop
Add-AppxPackage -Path $packageLiteral -ExternalLocation $locationLiteral -AllowUnsigned -ForceApplicationShutdown -ErrorAction Stop
if (-not (Get-AppxPackage -Name '$sharePackageName' -ErrorAction SilentlyContinue)) {
    throw 'Conduit share-target identity did not register'
}
"@
    Invoke-WindowsPowerShell $registerScript | Out-Null
    $shareTargetRegistered = $true
}

$shortcutSource = @'
using System;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
using System.Text;

[ComImport]
[Guid("00021401-0000-0000-C000-000000000046")]
internal class ShellLinkClass { }

[ComImport]
[Guid("000214F9-0000-0000-C000-000000000046")]
[InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IShellLinkW {
    void GetPath([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder file, int cch, IntPtr findData, uint flags);
    void GetIDList(out IntPtr pidl);
    void SetIDList(IntPtr pidl);
    void GetDescription([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder name, int cch);
    void SetDescription([MarshalAs(UnmanagedType.LPWStr)] string name);
    void GetWorkingDirectory([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder dir, int cch);
    void SetWorkingDirectory([MarshalAs(UnmanagedType.LPWStr)] string dir);
    void GetArguments([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder args, int cch);
    void SetArguments([MarshalAs(UnmanagedType.LPWStr)] string args);
    void GetHotkey(out short hotkey);
    void SetHotkey(short hotkey);
    void GetShowCmd(out int showCmd);
    void SetShowCmd(int showCmd);
    void GetIconLocation([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder iconPath, int cch, out int iconIndex);
    void SetIconLocation([MarshalAs(UnmanagedType.LPWStr)] string iconPath, int iconIndex);
    void SetRelativePath([MarshalAs(UnmanagedType.LPWStr)] string path, uint reserved);
    void Resolve(IntPtr hwnd, uint flags);
    void SetPath([MarshalAs(UnmanagedType.LPWStr)] string file);
}

[StructLayout(LayoutKind.Sequential, Pack = 4)]
internal struct PROPERTYKEY {
    public Guid fmtid;
    public uint pid;
    public PROPERTYKEY(Guid fmtid, uint pid) { this.fmtid = fmtid; this.pid = pid; }
}

[StructLayout(LayoutKind.Explicit)]
internal struct PROPVARIANT {
    [FieldOffset(0)] public ushort vt;
    [FieldOffset(8)] public IntPtr pointerValue;
}

[ComImport]
[Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99")]
[InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
internal interface IPropertyStore {
    [PreserveSig] int GetCount(out uint count);
    [PreserveSig] int GetAt(uint index, out PROPERTYKEY key);
    [PreserveSig] int GetValue(ref PROPERTYKEY key, out PROPVARIANT value);
    [PreserveSig] int SetValue(ref PROPERTYKEY key, ref PROPVARIANT value);
    [PreserveSig] int Commit();
}

public static class ConduitShortcut {
    [DllImport("shell32.dll")]
    private static extern void SHChangeNotify(uint eventId, uint flags, IntPtr item1, IntPtr item2);

    public static void Write(string shortcutPath, string exePath, string workingDir, string iconPath, string aumid) {
        var link = (IShellLinkW)new ShellLinkClass();
        link.SetPath(exePath);
        link.SetWorkingDirectory(workingDir);
        link.SetDescription("Conduit");
        link.SetIconLocation(iconPath, 0);
        link.SetShowCmd(1); // SW_SHOWNORMAL

        var store = (IPropertyStore)link;
        var key = new PROPERTYKEY(new Guid("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3"), 5); // PKEY_AppUserModel_ID
        var value = new PROPVARIANT { vt = 31, pointerValue = Marshal.StringToCoTaskMemUni(aumid) }; // VT_LPWSTR
        try {
            Marshal.ThrowExceptionForHR(store.SetValue(ref key, ref value));
            Marshal.ThrowExceptionForHR(store.Commit());
        } finally {
            Marshal.FreeCoTaskMem(value.pointerValue);
        }

        ((IPersistFile)link).Save(shortcutPath, true);
        SHChangeNotify(0x08000000, 0, IntPtr.Zero, IntPtr.Zero); // SHCNE_ASSOCCHANGED
    }
}
'@

if (-not ('ConduitShortcut' -as [type])) {
    Add-Type -TypeDefinition $shortcutSource -Language CSharp
}

$controlExe = Join-Path $installDir 'Conduit.exe'
$daemonExe = Join-Path $installDir 'conduit-daemon.exe'
$systemUsesLightTheme = (Get-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize' -Name SystemUsesLightTheme -ErrorAction SilentlyContinue).SystemUsesLightTheme
$lightShell = $null -eq $systemUsesLightTheme -or $systemUsesLightTheme -ne 0
$iconIco = Join-Path $installDir $(if ($lightShell) { 'conduit-icon-light.ico' } else { 'conduit-icon-dark.ico' })
$iconAumid = $iconIco
[ConduitShortcut]::Write($shortcut, $controlExe, $installDir, $iconIco, 'Conduit.Desktop')

New-Item -Force -Path $aumidKey | Out-Null
Set-ItemProperty -Path $aumidKey -Name DisplayName -Value 'Conduit'
Set-ItemProperty -Path $aumidKey -Name IconUri -Value $iconAumid
Set-ItemProperty -Path $aumidKey -Name IconBackgroundColor -Value '00000000'
Set-ItemProperty -Path $aumidKey -Name ShowInActionCenter -Type DWord -Value 1

if ($hadAutostart) {
    New-Item -Force -Path $runKey | Out-Null
    Set-ItemProperty -Path $runKey -Name Conduit -Value ('"{0}"' -f $daemonExe)
}

if ($hadExplorerIntegration) {
    $registration = Start-Process -FilePath $daemonExe -ArgumentList @('explorer', 'install') -Wait -PassThru -WindowStyle Hidden
    if ($registration.ExitCode -ne 0) {
        throw "Could not refresh Conduit Explorer integration (exit $($registration.ExitCode))"
    }
}

if (-not $NoStart) {
    Start-Process -FilePath $daemonExe -WorkingDirectory $installDir
}

[pscustomobject]@{
    InstallDir = $installDir
    Shortcut = $shortcut
    AppUserModelId = 'Conduit.Desktop'
    ShareTargetRegistered = $shareTargetRegistered
    AutostartPreserved = $hadAutostart
    ExplorerIntegrationPreserved = $hadExplorerIntegration
    DaemonStarted = -not $NoStart
}
