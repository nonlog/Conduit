using Conduit.Models;
using Conduit.ViewModels;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml.Media.Imaging;
using QRCoder;
using Windows.ApplicationModel.DataTransfer;
using Windows.ApplicationModel.DataTransfer.ShareTarget;
using Windows.Storage;
using Windows.Storage.Pickers;
using Windows.Storage.Streams;

namespace Conduit;

public sealed partial class MainPage : Page
{
    private bool _loaded;
    private CancellationTokenSource? _pairingExpiryRefresh;
    private CancellationTokenSource? _infoDismiss;
    private ContentDialog? _pairingDialog;
    private ShareOperation? _pendingShare;

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
            _ = HandleQueuedShareAsync();
        });
    }

    private void MainPage_Unloaded(object sender, RoutedEventArgs e)
    {
        _loaded = false;
        _pairingExpiryRefresh?.Cancel();
        _pairingExpiryRefresh?.Dispose();
        _pairingExpiryRefresh = null;
        _infoDismiss?.Cancel();
        _infoDismiss?.Dispose();
        _infoDismiss = null;
        try { _pairingDialog?.Hide(); } catch { }
        _pairingDialog = null;
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

    private async void PairDevice_Click(object sender, RoutedEventArgs e)
    {
        // Expiry is timestamp-based. Refresh before deciding so an elapsed window creates a fresh
        // six-digit code instead of reopening a stale dialog.
        ViewModel.RefreshAll();
        if (!ViewModel.PairingActive)
        {
            var result = ViewModel.StartPairing();
            if (!result.Success)
            {
                ShowInfo("Could not start pairing", result.Detail, InfoBarSeverity.Error);
                return;
            }
        }

        if (!ViewModel.PairingActive || ViewModel.PairingCode.Length != 6)
        {
            ShowInfo("Could not start pairing", "Conduit did not create a valid six-digit code.", InfoBarSeverity.Error);
            return;
        }

        SchedulePairingExpiryRefresh();
        await ShowPairingDialogAsync();
    }

    private async Task ShowPairingDialogAsync()
    {
        if (_pairingDialog is not null) return;

        var code = ViewModel.PairingCodeDisplay;
        var qr = await PairingQrAsync(code);
        var copy = new Button { Content = "Copy code" };
        var cancel = new Button { Content = "Cancel pairing" };
        var codeText = new TextBlock
        {
            Text = code,
            FontFamily = new Microsoft.UI.Xaml.Media.FontFamily("Consolas"),
            FontSize = 34,
            FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
        };
        var details = new StackPanel { Spacing = 12, VerticalAlignment = VerticalAlignment.Center };
        details.Children.Add(new TextBlock
        {
            Text = "Scan the QR code with your phone camera, or enter the six digits in Conduit.",
            TextWrapping = TextWrapping.Wrap,
            MaxWidth = 300,
        });
        details.Children.Add(codeText);
        details.Children.Add(new TextBlock
        {
            Text = "The code works through Conduit Relay and expires in two minutes.",
            TextWrapping = TextWrapping.Wrap,
            MaxWidth = 300,
            Foreground = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TextFillColorSecondaryBrush"],
        });
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        actions.Children.Add(copy);
        actions.Children.Add(cancel);
        details.Children.Add(actions);

        var content = new Grid { MinWidth = 550, ColumnSpacing = 24 };
        content.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(220) });
        content.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        var qrImage = new Image { Source = qr, Width = 220, Height = 220, Stretch = Microsoft.UI.Xaml.Media.Stretch.Uniform };
        content.Children.Add(qrImage);
        Grid.SetColumn(details, 1);
        content.Children.Add(details);

        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = "Pair phone",
            Content = content,
            CloseButtonText = "Done",
            DefaultButton = ContentDialogButton.Close,
        };
        _pairingDialog = dialog;

        copy.Click += (_, _) =>
        {
            var package = new DataPackage();
            package.SetText(code);
            Windows.ApplicationModel.DataTransfer.Clipboard.SetContent(package);
            Windows.ApplicationModel.DataTransfer.Clipboard.Flush();
            copy.Content = "Copied";
        };
        cancel.Click += (_, _) =>
        {
            var result = ViewModel.CancelPairing();
            _pairingExpiryRefresh?.Cancel();
            if (!result.Success)
                ShowInfo("Could not cancel pairing", result.Detail, InfoBarSeverity.Error);
            else
                ShowInfo("Pairing cancelled", "The existing paired phone was not changed.", InfoBarSeverity.Informational);
            dialog.Hide();
        };

        try
        {
            await dialog.ShowAsync();
        }
        finally
        {
            if (ReferenceEquals(_pairingDialog, dialog)) _pairingDialog = null;
        }
    }

    private static async Task<BitmapImage> PairingQrAsync(string code)
    {
        using var generator = new QRCodeGenerator();
        using var qrData = generator.CreateQrCode($"conduit://pair?code={code}", QRCodeGenerator.ECCLevel.Q);
        using var qrCode = new PngByteQRCode(qrData);
        var bytes = qrCode.GetGraphic(8);
        using var stream = new InMemoryRandomAccessStream();
        using (var writer = new DataWriter(stream))
        {
            writer.WriteBytes(bytes);
            await writer.StoreAsync();
            await writer.FlushAsync();
            writer.DetachStream();
        }
        stream.Seek(0);
        var bitmap = new BitmapImage();
        await bitmap.SetSourceAsync(stream);
        return bitmap;
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
            if (_loaded && !cancellation.IsCancellationRequested)
            {
                ViewModel.RefreshAll();
                if (!ViewModel.PairingActive)
                {
                    try { _pairingDialog?.Hide(); } catch { }
                }
            }
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

    public void QueueShare(ShareOperation operation)
    {
        _pendingShare = operation;
        if (_loaded) _ = HandleQueuedShareAsync();
    }

    private async Task HandleQueuedShareAsync()
    {
        var operation = _pendingShare;
        if (operation is null) return;
        _pendingShare = null;
        operation.ReportStarted();
        try
        {
            if (!operation.Data.Contains(StandardDataFormats.StorageItems))
            {
                operation.ReportError("Conduit can receive files from the Windows share sheet.");
                ShowInfo("Could not receive shared item", "The shared content did not contain a file.", InfoBarSeverity.Error);
                return;
            }

            var items = await operation.Data.GetStorageItemsAsync();
            var files = items.OfType<StorageFile>().ToList();
            operation.ReportDataRetrieved();
            if (files.Count == 0)
            {
                operation.ReportError("The shared item did not contain a file.");
                ShowInfo("Could not receive shared item", "No file was available to send.", InfoBarSeverity.Error);
                return;
            }

            SendFileButton.IsEnabled = false;
            var failures = new List<string>();
            try
            {
                for (var index = 0; index < files.Count; index++)
                {
                    var file = files[index];
                    ShowInfo(
                        "Sending shared file",
                        files.Count == 1 ? file.Name : $"{index + 1} of {files.Count}: {file.Name}",
                        InfoBarSeverity.Informational);
                    var result = await ViewModel.SendFileAsync(file.Path);
                    if (!result.Success) failures.Add(string.IsNullOrWhiteSpace(result.Detail) ? file.Name : $"{file.Name}: {result.Detail}");
                }
            }
            finally
            {
                SendFileButton.IsEnabled = true;
                ViewModel.RefreshAll();
            }

            if (failures.Count > 0)
            {
                var detail = string.Join("; ", failures.Take(3));
                operation.ReportError(detail);
                ShowInfo("Could not send all shared files", detail, InfoBarSeverity.Error);
                return;
            }

            operation.ReportCompleted();
            ShowInfo(
                "Shared to phone",
                files.Count == 1 ? $"{files[0].Name} was sent." : $"{files.Count} files were sent.",
                InfoBarSeverity.Success);
        }
        catch (Exception ex)
        {
            try { operation.ReportError(ex.Message); } catch { }
            ShowInfo("Could not send shared file", ex.Message, InfoBarSeverity.Error);
        }
    }

    private void ShowInfo(string title, string message, InfoBarSeverity severity)
    {
        _infoDismiss?.Cancel();
        _infoDismiss?.Dispose();
        StatusInfoBar.Title = title;
        StatusInfoBar.Message = message;
        StatusInfoBar.Severity = severity;
        StatusInfoBar.IsOpen = true;

        var cancellation = new CancellationTokenSource();
        _infoDismiss = cancellation;
        var delay = severity switch
        {
            InfoBarSeverity.Error => TimeSpan.FromSeconds(8),
            InfoBarSeverity.Warning => TimeSpan.FromSeconds(6),
            _ => TimeSpan.FromSeconds(4),
        };
        _ = DismissInfoAsync(cancellation, delay);
    }

    private async Task DismissInfoAsync(CancellationTokenSource cancellation, TimeSpan delay)
    {
        try
        {
            await Task.Delay(delay, cancellation.Token);
            if (_loaded && ReferenceEquals(_infoDismiss, cancellation)) StatusInfoBar.IsOpen = false;
        }
        catch (OperationCanceledException) { }
        finally
        {
            if (ReferenceEquals(_infoDismiss, cancellation))
            {
                _infoDismiss = null;
                cancellation.Dispose();
            }
        }
    }
}
