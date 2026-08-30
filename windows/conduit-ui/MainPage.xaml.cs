using Conduit.Models;
using Conduit.ViewModels;
using Windows.Storage.Pickers;

namespace Conduit;

public sealed partial class MainPage : Page
{
    private bool _loaded;

    public MainViewModel ViewModel { get; } = new();

    public MainPage()
    {
        InitializeComponent();
        DataContext = ViewModel;
    }

    private void MainPage_Loaded(object sender, RoutedEventArgs e)
    {
        App.MainWindow.SetTitleBar(TitleBarRoot);
        MainNavigationView.SelectedItem = SharedLinksNavigationItem;
        SettingsNavigation.SelectedItem = GeneralSettingsNavigationItem;
        ViewModel.Initialize(DispatcherQueue);
        _loaded = true;

        if (!string.IsNullOrWhiteSpace(ViewModel.StartupSendError))
            ShowInfo("Could not send file", ViewModel.StartupSendError, InfoBarSeverity.Error);
    }

    private void MainPage_Unloaded(object sender, RoutedEventArgs e)
    {
        _loaded = false;
        ViewModel.Dispose();
    }

    private void Refresh_Click(object sender, RoutedEventArgs e)
    {
        ViewModel.RefreshAll();
        ShowInfo("Status refreshed", $"{ViewModel.ConnectionState} · {ViewModel.ConnectionRoute}", InfoBarSeverity.Informational);
    }

    private async void SendFile_Click(object sender, RoutedEventArgs e)
    {
        var picker = new FileOpenPicker
        {
            SuggestedStartLocation = PickerLocationId.DocumentsLibrary,
            ViewMode = PickerViewMode.List,
        };
        picker.FileTypeFilter.Add("*");
        InitializePicker(picker);

        var file = await picker.PickSingleFileAsync();
        if (file is null) return;

        SendFileButton.IsEnabled = false;
        ShowInfo("Sending file", $"{file.Name}  →  {ViewModel.DeviceName}", InfoBarSeverity.Informational);
        try
        {
            var result = await ViewModel.SendFileAsync(file.Path);
            if (result.Success)
                ShowInfo("Sent to phone", $"{file.Name} was received by {ViewModel.DeviceName}.", InfoBarSeverity.Success);
            else
                ShowInfo("Could not send file", string.IsNullOrWhiteSpace(result.Detail) ? file.Name : $"{file.Name}\n{result.Detail}", InfoBarSeverity.Error);
        }
        finally
        {
            SendFileButton.IsEnabled = true;
            ViewModel.RefreshAll();
        }
    }

    private static void InitializePicker(object picker)
    {
        var hwnd = WinRT.Interop.WindowNative.GetWindowHandle(App.MainWindow);
        WinRT.Interop.InitializeWithWindow.Initialize(picker, hwnd);
    }

    private void ShowSharedLinks_Click(object sender, RoutedEventArgs e)
    {
        MainNavigationView.SelectedItem = SharedLinksNavigationItem;
        ShowSharedLinksPage();
    }

    private void ShowSettings_Click(object sender, RoutedEventArgs e)
    {
        if (MainNavigationView.SettingsItem is not null)
            MainNavigationView.SelectedItem = MainNavigationView.SettingsItem;
        ShowSettingsPage();
    }

    private void MainNavigationView_SelectionChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        if (args.IsSettingsSelected) ShowSettingsPage();
        else ShowSharedLinksPage();
    }

    private void ShowSharedLinksPage()
    {
        SharedLinksPage.Visibility = Visibility.Visible;
        SettingsPage.Visibility = Visibility.Collapsed;
    }

    private void ShowSettingsPage()
    {
        SharedLinksPage.Visibility = Visibility.Collapsed;
        SettingsPage.Visibility = Visibility.Visible;
    }

    private void SettingsNavigation_SelectionChanged(NavigationView sender, NavigationViewSelectionChangedEventArgs args)
    {
        var tag = args.SelectedItemContainer?.Tag?.ToString();
        var about = string.Equals(tag, "About", StringComparison.OrdinalIgnoreCase);
        GeneralSettingsPanel.Visibility = about ? Visibility.Collapsed : Visibility.Visible;
        AboutSettingsPanel.Visibility = about ? Visibility.Visible : Visibility.Collapsed;
    }

    private void SharedLinksList_ItemClick(object sender, ItemClickEventArgs e)
    {
        if (e.ClickedItem is not SharedLinkEntry link) return;
        if (!Uri.TryCreate(link.Url, UriKind.Absolute, out var uri) ||
            (uri.Scheme != Uri.UriSchemeHttp && uri.Scheme != Uri.UriSchemeHttps)) return;
        Process.Start(new ProcessStartInfo(link.Url) { UseShellExecute = true });
    }

    private void ClearNotifications_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            ViewModel.ClearNotifications();
            ShowInfo("Notification history cleared", "Local mirrored-notification history was removed.", InfoBarSeverity.Success);
        }
        catch (Exception ex)
        {
            ShowInfo("Could not clear notifications", ex.Message, InfoBarSeverity.Error);
        }
    }

    private async void ClearLinks_Click(object sender, RoutedEventArgs e)
    {
        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = "Clear shared-link history?",
            Content = "This removes the local history from this PC.",
            PrimaryButtonText = "Clear",
            CloseButtonText = "Cancel",
            DefaultButton = ContentDialogButton.Close,
        };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;

        try
        {
            ViewModel.ClearLinks();
            ShowInfo("Shared-link history cleared", "Local shared links were removed.", InfoBarSeverity.Success);
        }
        catch (Exception ex)
        {
            ShowInfo("Could not clear shared links", ex.Message, InfoBarSeverity.Error);
        }
    }

    private void SaveSettings_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            var applied = ViewModel.SaveSettings();
            SaveStateText.Text = applied ? "Applied" : "Saved";
            ShowInfo(
                applied ? "Settings applied" : "Settings saved",
                applied ? "The daemon reloaded the new settings." : "The daemon will use them on its next start.",
                applied ? InfoBarSeverity.Success : InfoBarSeverity.Warning);
        }
        catch (Exception ex)
        {
            ShowInfo("Could not save settings", ex.Message, InfoBarSeverity.Error);
        }
    }

    private void AutostartToggle_Toggled(object sender, RoutedEventArgs e)
    {
        if (!_loaded || sender is not ToggleSwitch toggle || toggle.IsOn == ViewModel.StartAtSignIn) return;
        var wanted = toggle.IsOn;
        if (!ViewModel.SetAutostart(wanted))
            ShowInfo("Could not update startup", "The Start at sign-in setting was not changed.", InfoBarSeverity.Error);
        else
            ShowInfo("Startup setting updated", wanted ? "Conduit will start when you sign in." : "Conduit will no longer start at sign-in.", InfoBarSeverity.Success);
    }

    private void ExplorerToggle_Toggled(object sender, RoutedEventArgs e)
    {
        if (!_loaded || sender is not ToggleSwitch toggle || toggle.IsOn == ViewModel.ExplorerIntegration) return;
        var wanted = toggle.IsOn;
        if (!ViewModel.SetExplorerIntegration(wanted))
            ShowInfo("Could not update Explorer", "The Send with Conduit command was not changed.", InfoBarSeverity.Error);
        else
            ShowInfo("Explorer integration updated", wanted ? "Send with Conduit is available in Explorer." : "Send with Conduit was removed from Explorer.", InfoBarSeverity.Success);
    }

    private async void SelectReceiveFolder_Click(object sender, RoutedEventArgs e)
    {
        var picker = new FolderPicker
        {
            SuggestedStartLocation = PickerLocationId.Downloads,
            ViewMode = PickerViewMode.List,
        };
        picker.FileTypeFilter.Add("*");
        InitializePicker(picker);
        var folder = await picker.PickSingleFolderAsync();
        if (folder is null) return;
        ViewModel.SetReceiveFolder(folder.Path);
        SaveStateText.Text = "Unsaved changes";
    }

    private void ResetReceiveFolder_Click(object sender, RoutedEventArgs e)
    {
        ViewModel.SetReceiveFolder(null);
        SaveStateText.Text = "Unsaved changes";
    }

    private void OpenDataFolder_Click(object sender, RoutedEventArgs e) => ViewModel.OpenDataFolder();

    private void OpenRepository_Click(object sender, RoutedEventArgs e) =>
        Process.Start(new ProcessStartInfo("https://github.com/nonlog/Conduit") { UseShellExecute = true });

    private void OpenDiagnostics_Click(object sender, RoutedEventArgs e) =>
        ShowInfo("Conduit diagnostics", ViewModel.Diagnostics(), InfoBarSeverity.Informational);

    private void ShowInfo(string title, string message, InfoBarSeverity severity)
    {
        StatusInfoBar.Title = title;
        StatusInfoBar.Message = message;
        StatusInfoBar.Severity = severity;
        StatusInfoBar.IsOpen = true;
    }
}
