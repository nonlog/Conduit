using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml.Media;
using Windows.Graphics;

namespace Conduit;

public partial class App : Application
{
    public static Window MainWindow { get; private set; } = null!;

    public App()
    {
        InitializeComponent();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        MainWindow = new Window
        {
            Title = "Conduit",
            ExtendsContentIntoTitleBar = true,
        };

        try { MainWindow.SystemBackdrop = new MicaBackdrop(); } catch { }

        MainWindow.Content = new MainPage();

        var icon = Path.Combine(AppContext.BaseDirectory, "Assets", "conduit-icon.ico");
        if (File.Exists(icon))
        {
            try { MainWindow.AppWindow.SetIcon(icon); } catch { }
        }

        try
        {
            MainWindow.AppWindow.Resize(new SizeInt32(1280, 760));
        }
        catch { }

        MainWindow.Activate();
    }
}
