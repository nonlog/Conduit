using Microsoft.UI.Windowing;
using Windows.ApplicationModel;
using Windows.ApplicationModel.Activation;
using Microsoft.UI.Xaml.Media;
using Microsoft.Win32;
using Windows.Graphics;
using System.Threading;

namespace Conduit;

public partial class App : Application
{
    public static Window MainWindow { get; private set; } = null!;
    private readonly Windows.UI.ViewManagement.UISettings _uiSettings = new();
    private Mutex? _singleInstanceMutex;
    private CancellationTokenSource? _shellThemeRefresh;

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

        // The resident tray can receive another double-click while the self-contained WinUI process
        // is still cold-starting. Keep a process-wide named mutex as a second line of defence so a
        // launch race can never turn into a stack of Conduit windows. ShareTarget activations stay
        // independent so Windows can hand us their ShareOperation without dropping the payload.
        if (activation?.Kind != ActivationKind.ShareTarget && !TryOwnControlWindow())
        {
            TaskbarIdentity.ActivateExistingWindow();
            Environment.Exit(0);
            return;
        }

        MainWindow = new Window
        {
            Title = "Conduit",
            ExtendsContentIntoTitleBar = true,
        };

        try { MainWindow.SystemBackdrop = new MicaBackdrop(); } catch { }

        var mainPage = new MainPage();
        mainPage.ActualThemeChanged += (_, _) => ApplyWindowTheme(mainPage.ActualTheme);
        // UISettings can arrive slightly before the Personalize registry values and Explorer have
        // finished switching palettes. Apply immediately, then once more after a short one-shot
        // settle delay. This is event-driven; there is no theme polling loop.
        _uiSettings.ColorValuesChanged += (_, _) => ScheduleShellThemeRefresh();
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
        try
        {
            MainWindow.AppWindow.Resize(new SizeInt32(1280, 760));
        }
        catch { }

        MainWindow.Activate();
        // Package/Shell identity is finalized when the first taskbar button is created. Reapply
        // the themed Win32 icon only after activation so the sparse ShareTarget identity cannot
        // replace a dark-taskbar white icon with the executable's default black icon.
        ApplyWindowTheme(mainPage.ActualTheme);
        MainWindow.DispatcherQueue.TryEnqueue(() => ApplyWindowTheme(mainPage.ActualTheme));
    }

    private bool TryOwnControlWindow()
    {
        try
        {
            _singleInstanceMutex = new Mutex(true, @"Local\Conduit.Desktop.ControlWindow", out var createdNew);
            return createdNew;
        }
        catch
        {
            // If the mutex API itself is unavailable, preserve the ordinary launch path rather than
            // making the control surface inaccessible. The tray-side launch gate still applies.
            return true;
        }
    }

    private void ScheduleShellThemeRefresh()
    {
        var previous = Interlocked.Exchange(ref _shellThemeRefresh, new CancellationTokenSource());
        previous?.Cancel();
        previous?.Dispose();
        var refresh = _shellThemeRefresh;
        if (refresh is null) return;

        MainWindow.DispatcherQueue.TryEnqueue(ApplyThemeIcon);
        _ = ReapplyShellThemeAfterSettleAsync(refresh);
    }

    private async Task ReapplyShellThemeAfterSettleAsync(CancellationTokenSource refresh)
    {
        try
        {
            await Task.Delay(TimeSpan.FromMilliseconds(600), refresh.Token);
            if (!refresh.IsCancellationRequested)
                MainWindow.DispatcherQueue.TryEnqueue(ApplyThemeIcon);
        }
        catch (OperationCanceledException) { }
        finally
        {
            if (ReferenceEquals(Interlocked.CompareExchange(ref _shellThemeRefresh, null, refresh), refresh))
                refresh.Dispose();
        }
    }

    internal static void ApplyWindowTheme(ElementTheme theme)
    {
        ApplyThemeIcon();
        ApplyCaptionButtonTheme(theme);
    }

    private static void ApplyCaptionButtonTheme(ElementTheme theme)
    {
        try
        {
            var dark = theme == ElementTheme.Dark;
            var titleBar = MainWindow.AppWindow.TitleBar;
            var foreground = dark ? Microsoft.UI.Colors.White : Microsoft.UI.Colors.Black;
            var inactiveForeground = dark
                ? Windows.UI.Color.FromArgb(0x99, 0xff, 0xff, 0xff)
                : Windows.UI.Color.FromArgb(0x99, 0x00, 0x00, 0x00);
            var hoverBackground = dark
                ? Windows.UI.Color.FromArgb(0x18, 0xff, 0xff, 0xff)
                : Windows.UI.Color.FromArgb(0x12, 0x00, 0x00, 0x00);
            var pressedBackground = dark
                ? Windows.UI.Color.FromArgb(0x28, 0xff, 0xff, 0xff)
                : Windows.UI.Color.FromArgb(0x20, 0x00, 0x00, 0x00);

            titleBar.ButtonForegroundColor = foreground;
            titleBar.ButtonInactiveForegroundColor = inactiveForeground;
            titleBar.ButtonHoverForegroundColor = foreground;
            titleBar.ButtonPressedForegroundColor = foreground;
            titleBar.ButtonBackgroundColor = Microsoft.UI.Colors.Transparent;
            titleBar.ButtonInactiveBackgroundColor = Microsoft.UI.Colors.Transparent;
            titleBar.ButtonHoverBackgroundColor = hoverBackground;
            titleBar.ButtonPressedBackgroundColor = pressedBackground;
        }
        catch { }
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
