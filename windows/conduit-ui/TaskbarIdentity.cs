using Microsoft.UI;
using Microsoft.UI.Xaml;
using System.Runtime.InteropServices;

namespace Conduit;

internal static class TaskbarIdentity
{
    private const string AppUserModelId = "Conduit.Desktop";
    private const ushort VtLpwstr = 31;
    private static readonly Guid AppUserModelFormat = new("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3");

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
