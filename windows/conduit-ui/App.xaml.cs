using Microsoft.UI.Windowing;
using Windows.ApplicationModel;
using Windows.ApplicationModel.Activation;
using Microsoft.UI.Xaml.Media;
using Microsoft.Win32;
using Windows.Graphics;

namespace Conduit;

public partial class App : Application
{
    public static Window MainWindow { get; private set; } = null!;

    public App()
    {
        // Keep the WinUI window on Conduit's long-standing Win32 taskbar identity even when the
        // sparse ShareTarget package grants this process package identity.
        TaskbarIdentity.SetProcessIdentity();
        InitializeComponent();
    }

    protected override void OnLaunched(Microsoft.UI.Xaml.LaunchActivatedEventArgs args)
    {
        // Packaged activations are one-shot. Capture the share payload before WinUI/Uno startup
        // can observe activation state, then hand it to the page after the shell exists.
        IActivatedEventArgs? activation = null;
        try { activation = Windows.ApplicationModel.AppInstance.GetActivatedEventArgs(); } catch { }

        MainWindow = new Window
        {
            Title = "Conduit",
            ExtendsContentIntoTitleBar = true,
        };

        try { MainWindow.SystemBackdrop = new MicaBackdrop(); } catch { }

        var mainPage = new MainPage();
        mainPage.ActualThemeChanged += (_, _) => ApplyThemeIcon();
        MainWindow.Content = mainPage;

        try
        {
            if (activation?.Kind == ActivationKind.ShareTarget &&
                activation is ShareTargetActivatedEventArgs share)
            {
                mainPage.QueueShare(share.ShareOperation);
            }
        }
        catch
        {
            // A normal unpackaged launch has no package activation context. Share-target identity
            // is optional, so falling back to the ordinary control window is intentional.
        }
        ApplyThemeIcon();

        try
        {
            MainWindow.AppWindow.Resize(new SizeInt32(1280, 760));
        }
        catch { }

        MainWindow.Activate();
    }

    private static void ApplyThemeIcon()
    {
        var icon = Path.Combine(AppContext.BaseDirectory, "Assets", ThemeIconFileName());
        if (!File.Exists(icon))
        {
            icon = Path.Combine(AppContext.BaseDirectory, "Assets", "conduit-icon.ico");
        }
        if (File.Exists(icon))
        {
            try { MainWindow.AppWindow.SetIcon(icon); } catch { }
            TaskbarIdentity.Apply(MainWindow, icon);
        }
    }

    private static string ThemeIconFileName()
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(
                @"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
            if (key?.GetValue("SystemUsesLightTheme") is int light && light == 0)
            {
                return "conduit-icon-dark.ico";
            }
        }
        catch { }
        return "conduit-icon-light.ico";
    }
}
