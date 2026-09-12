using Microsoft.UI;
using Microsoft.UI.Xaml;
using System.Runtime.InteropServices;

namespace Conduit;

internal static class TaskbarIdentity
{
    private const string AppUserModelId = "Conduit.Desktop";
    private const ushort VtLpwstr = 31;
    private static readonly Guid AppUserModelFormat = new("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3");
    private const uint WmSetIcon = 0x0080;
    private const int IconSmall = 0;
    private const int IconBig = 1;
    private const uint ImageIcon = 1;
    private const uint LrLoadFromFile = 0x0010;
    private const int SmCxIcon = 11;
    private const int SmCyIcon = 12;
    private const int SmCxSmIcon = 49;
    private const int SmCySmIcon = 50;
    private static IntPtr _smallIcon;
    private static IntPtr _bigIcon;

    public static void SetProcessIdentity()
    {
        try { _ = SetCurrentProcessExplicitAppUserModelID(AppUserModelId); } catch { }
    }

    public static void Apply(Window window, string iconPath)
    {
        if (string.IsNullOrWhiteSpace(iconPath) || !File.Exists(iconPath)) return;

        try
        {
            var hwnd = Win32Interop.GetWindowFromWindowId(window.AppWindow.Id);
            if (hwnd == IntPtr.Zero) return;

            ApplyWindowIcons(hwnd, iconPath);

            var iid = typeof(IPropertyStore).GUID;
            Marshal.ThrowExceptionForHR(SHGetPropertyStoreForWindow(hwnd, ref iid, out var store));
            try
            {
                var processPath = Environment.ProcessPath;
                if (!string.IsNullOrWhiteSpace(processPath))
                {
                    SetString(store, 2, $"\"{processPath}\""); // PKEY_AppUserModel_RelaunchCommand
                }
                SetString(store, 3, $"{iconPath},0"); // PKEY_AppUserModel_RelaunchIconResource
                SetString(store, 5, AppUserModelId); // PKEY_AppUserModel_ID; set last so Shell refreshes
                Marshal.ThrowExceptionForHR(store.Commit());
            }
            finally
            {
                if (Marshal.IsComObject(store)) Marshal.FinalReleaseComObject(store);
            }
        }
        catch
        {
            // Package identity is optional for normal desktop launches. If Shell interop is
            // unavailable, AppWindow.SetIcon still supplies the ordinary Win32 window icon.
        }
    }

    private static void ApplyWindowIcons(IntPtr hwnd, string iconPath)
    {
        var small = LoadImage(IntPtr.Zero, iconPath, ImageIcon, GetSystemMetrics(SmCxSmIcon), GetSystemMetrics(SmCySmIcon), LrLoadFromFile);
        var big = LoadImage(IntPtr.Zero, iconPath, ImageIcon, GetSystemMetrics(SmCxIcon), GetSystemMetrics(SmCyIcon), LrLoadFromFile);
        if (small != IntPtr.Zero)
        {
            _ = SendMessage(hwnd, WmSetIcon, (IntPtr)IconSmall, small);
            var old = Interlocked.Exchange(ref _smallIcon, small);
            if (old != IntPtr.Zero && old != small) _ = DestroyIcon(old);
        }
        if (big != IntPtr.Zero)
        {
            _ = SendMessage(hwnd, WmSetIcon, (IntPtr)IconBig, big);
            var old = Interlocked.Exchange(ref _bigIcon, big);
            if (old != IntPtr.Zero && old != big) _ = DestroyIcon(old);
        }
    }

    private static void SetString(IPropertyStore store, uint propertyId, string value)
    {
        var key = new PropertyKey(AppUserModelFormat, propertyId);
        var variant = new PropVariant
        {
            VariantType = VtLpwstr,
            PointerValue = Marshal.StringToCoTaskMemUni(value),
        };
        try
        {
            Marshal.ThrowExceptionForHR(store.SetValue(ref key, ref variant));
        }
        finally
        {
            Marshal.FreeCoTaskMem(variant.PointerValue);
        }
    }

    [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
    private static extern int SetCurrentProcessExplicitAppUserModelID(string appId);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr LoadImage(IntPtr instance, string name, uint type, int width, int height, uint flags);

    [DllImport("user32.dll")]
    private static extern IntPtr SendMessage(IntPtr hwnd, uint message, IntPtr wParam, IntPtr lParam);

    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DestroyIcon(IntPtr icon);

    [DllImport("user32.dll")]
    private static extern int GetSystemMetrics(int index);

    [DllImport("shell32.dll")]
    private static extern int SHGetPropertyStoreForWindow(
        IntPtr hwnd,
        ref Guid riid,
        [MarshalAs(UnmanagedType.Interface)] out IPropertyStore propertyStore);

    [StructLayout(LayoutKind.Sequential, Pack = 4)]
    private struct PropertyKey
    {
        public Guid FormatId;
        public uint PropertyId;

        public PropertyKey(Guid formatId, uint propertyId)
        {
            FormatId = formatId;
            PropertyId = propertyId;
        }
    }

    [StructLayout(LayoutKind.Explicit)]
    private struct PropVariant
    {
        [FieldOffset(0)] public ushort VariantType;
        [FieldOffset(8)] public IntPtr PointerValue;
    }

    [ComImport]
    [Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IPropertyStore
    {
        [PreserveSig] int GetCount(out uint count);
        [PreserveSig] int GetAt(uint index, out PropertyKey key);
        [PreserveSig] int GetValue(ref PropertyKey key, out PropVariant value);
        [PreserveSig] int SetValue(ref PropertyKey key, ref PropVariant value);
        [PreserveSig] int Commit();
    }
}
