using Conduit.Models;
using Conduit.ViewModels;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage;
using Windows.Storage.Pickers;

namespace Conduit;

public sealed partial class MainPage : Page
{
    private bool _loaded;
    private CancellationTokenSource? _pairingExpiryRefresh;

    public MainViewModel ViewModel { get; } = new();

    public MainPage()
    {
        InitializeComponent();
        DataContext = ViewModel;
        ActualThemeChanged += (_, _) => UpdateTitleBarIcon();
    }

    private void UpdateTitleBarIcon()
    {
        var file = ActualTheme == ElementTheme.Dark ? "conduit-icon-dark.png" : "conduit-icon-light.png";
        TitleBarIcon.Source = new BitmapImage(new Uri($"ms-appx:///Assets/{file}"));
    }

    private void MainPage_Loaded(object sender, RoutedEventArgs e)
    {
        UpdateTitleBarIcon();
        App.MainWindow.SetTitleBar(TitleBarRoot);
        MainNavigationView.SelectedItem = SharedLinksNavigationItem;
        SettingsNavigation.SelectedItem = GeneralSettingsNavigationItem;
        _loaded = true;
        // Present the shell first. Status/config/history I/O is useful, but none of it should delay
        // the first frame when the resident tray launches this on-demand UI.
        DispatcherQueue.TryEnqueue(DispatcherQueuePriority.Low, () =>
        {
            if (!_loaded) return;
            ViewModel.Initialize(DispatcherQueue);
            if (!string.IsNullOrWhiteSpace(ViewModel.StartupSendError))
                ShowInfo("Could not send file", ViewModel.StartupSendError, InfoBarSeverity.Error);
        });
    }

    private void MainPage_Unloaded(object sender, RoutedEventArgs e)
    {
        _loaded = false;
        _pairingExpiryRefresh?.Cancel();
        _pairingExpiryRefresh?.Dispose();
        _pairingExpiryRefresh = null;
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

    private void DropZone_DragOver(object sender, DragEventArgs e)
    {
        if (e.DataView.Contains(StandardDataFormats.StorageItems))
        {
            e.AcceptedOperation = DataPackageOperation.Copy;
            e.DragUIOverride.Caption = $"Send to {ViewModel.DeviceName}";
            e.DragUIOverride.IsCaptionVisible = true;
            e.DragUIOverride.IsContentVisible = true;
        }
    }

    private async void DropZone_Drop(object sender, DragEventArgs e)
    {
        if (!e.DataView.Contains(StandardDataFormats.StorageItems)) return;
        var items = await e.DataView.GetStorageItemsAsync();
        var files = items.OfType<StorageFile>().ToList();
        if (files.Count == 0) return;

        SendFileButton.IsEnabled = false;
        var failures = new List<string>();
        try
        {
            for (var index = 0; index < files.Count; index++)
            {
                var file = files[index];
                var progress = files.Count == 1 ? file.Name : $"{index + 1} of {files.Count}: {file.Name}";
                ShowInfo("Sending file", $"{progress}  →  {ViewModel.DeviceName}", InfoBarSeverity.Informational);

                var result = await ViewModel.SendFileAsync(file.Path);
                if (!result.Success)
                {
                    var detail = string.IsNullOrWhiteSpace(result.Detail) ? file.Name : $"{file.Name}: {result.Detail}";
                    failures.Add(detail);
                }
            }

            if (failures.Count == 0)
            {
                var summary = files.Count == 1 ? files[0].Name : $"{files.Count} files";
                ShowInfo("Sent to phone", $"{summary} received by {ViewModel.DeviceName}.", InfoBarSeverity.Success);
            }
            else
            {
                ShowInfo(
                    "Could not send all files",
                    $"{failures.Count} of {files.Count} failed. {string.Join("; ", failures.Take(3))}",
                    InfoBarSeverity.Error);
            }
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

    private void PairDevice_Click(object sender, RoutedEventArgs e)
    {
        // Expiry is timestamp-based. Refresh before deciding so a window that elapsed while the UI
        // was open cannot turn the next Pair click into a stale Cancel operation.
        ViewModel.RefreshAll();
        var result = ViewModel.PairingActive ? ViewModel.CancelPairing() : ViewModel.StartPairing();
        if (!result.Success)
        {
            ShowInfo("Could not update pairing", result.Detail, InfoBarSeverity.Error);
            return;
        }

        if (ViewModel.PairingActive)
        {
            ShowInfo(
                "Pairing is open",
                $"Enter {ViewModel.PairingCodeDisplay} on the phone. The temporary code works through Conduit Relay even when the devices are on different networks, and expires in two minutes.",
                InfoBarSeverity.Informational);
            SchedulePairingExpiryRefresh();
        }
        else
        {
            _pairingExpiryRefresh?.Cancel();
            ShowInfo("Pairing cancelled", "The existing paired phone was not changed.", InfoBarSeverity.Informational);
        }
    }

    private void CopyPairingCode_Click(object sender, RoutedEventArgs e)
    {
        if (!ViewModel.PairingActive || string.IsNullOrWhiteSpace(ViewModel.PairingCode)) return;
        var package = new Windows.ApplicationModel.DataTransfer.DataPackage();
        package.SetText(ViewModel.PairingCodeDisplay);
        Windows.ApplicationModel.DataTransfer.Clipboard.SetContent(package);
        Windows.ApplicationModel.DataTransfer.Clipboard.Flush();
        ShowInfo("Pairing code copied", "Enter it in Conduit on the phone before the two-minute window expires.", InfoBarSeverity.Success);
    }

    private async void ForgetDevice_Click(object sender, RoutedEventArgs e)
    {
        if (!ViewModel.HasPairedDevice) return;
        var name = string.IsNullOrWhiteSpace(ViewModel.PairedDeviceName) ? "this phone" : ViewModel.PairedDeviceName;
        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = $"Forget {name}?",
            Content = "This removes the pairing from this PC and tells the connected phone to forget it too. The PC identity and your Conduit settings are kept.",
            PrimaryButtonText = "Forget",
            CloseButtonText = "Cancel",
            DefaultButton = ContentDialogButton.Close,
        };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;

        var result = ViewModel.ForgetPairedDevice();
        if (result.Success)
            ShowInfo("Phone forgotten", "Pairing was removed. Use Pair phone on both devices to link again.", InfoBarSeverity.Success);
        else
            ShowInfo("Could not forget phone", result.Detail, InfoBarSeverity.Error);
    }

    private async void SchedulePairingExpiryRefresh()
    {
        _pairingExpiryRefresh?.Cancel();
        _pairingExpiryRefresh?.Dispose();
        var cancellation = new CancellationTokenSource();
        _pairingExpiryRefresh = cancellation;
        try
        {
            await Task.Delay(TimeSpan.FromSeconds(121), cancellation.Token);
            if (_loaded && !cancellation.IsCancellationRequested) ViewModel.RefreshAll();
        }
        catch (OperationCanceledException) { }
        finally
        {
            if (ReferenceEquals(_pairingExpiryRefresh, cancellation))
            {
                _pairingExpiryRefresh = null;
                cancellation.Dispose();
            }
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
