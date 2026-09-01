using Conduit.Models;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.Win32;

namespace Conduit.ViewModels;

public sealed class MainViewModel : ObservableObject, IDisposable
{
    private readonly string _dataDir;
    private DispatcherQueue? _dispatcher;
    private FileSystemWatcher? _dataWatcher;
    private bool _loading;
    private string? _receiveDir;

    private string _deviceName = "No phone";
    private string _connectionState = "Disconnected";
    private string _connectionRoute = "—";
    private string _connectionGlyph = "\uE711";
    private ImageSource? _wallpaperSource;
    private bool _isLinked;
    private bool _relayUs = true;
    private bool _relayWa = true;
    private bool _relayTyo = true;
    private bool _relayJp = true;
    private string _proxyMode = "System proxy";
    private string _manualProxy = string.Empty;
    private string _systemProxyDescription = "Windows system proxy is off";
    private string _receiveFolder = string.Empty;
    private bool _trayIcon;
    private bool _startAtSignIn;
    private bool _explorerIntegration;
    private bool _hasNotifications;
    private bool _hasLinks;

    public ObservableCollection<NotificationEntry> Notifications { get; } = [];
    public ObservableCollection<SharedLinkEntry> SharedLinks { get; } = [];

    public string DeviceName { get => _deviceName; private set => SetProperty(ref _deviceName, value); }
    public string ConnectionState { get => _connectionState; private set => SetProperty(ref _connectionState, value); }
    public string ConnectionRoute { get => _connectionRoute; private set => SetProperty(ref _connectionRoute, value); }
    public string ConnectionGlyph { get => _connectionGlyph; private set => SetProperty(ref _connectionGlyph, value); }
    public ImageSource? WallpaperSource { get => _wallpaperSource; private set => SetProperty(ref _wallpaperSource, value); }
    public bool IsLinked { get => _isLinked; private set => SetProperty(ref _isLinked, value); }
    public bool RelayUs { get => _relayUs; set => SetRelay(ref _relayUs, value); }
    public bool RelayWa { get => _relayWa; set => SetRelay(ref _relayWa, value); }
    public bool RelayTyo { get => _relayTyo; set => SetRelay(ref _relayTyo, value); }
    public bool RelayJp { get => _relayJp; set => SetRelay(ref _relayJp, value); }
    public string RelaySummary => $"{EnabledRelayCount} of 4 relay points enabled";
    public ObservableCollection<string> ProxyModes { get; } = ["System proxy", "Manual SOCKS5", "Direct"];
    public string ProxyMode
    {
        get => _proxyMode;
        set
        {
            if (!SetProperty(ref _proxyMode, value)) return;
            OnPropertyChanged(nameof(ManualProxyEnabled));
        }
    }
    public string ManualProxy { get => _manualProxy; set => SetProperty(ref _manualProxy, value); }
    public bool ManualProxyEnabled => ProxyMode.Equals("Manual SOCKS5", StringComparison.Ordinal);
    public string SystemProxyDescription { get => _systemProxyDescription; private set => SetProperty(ref _systemProxyDescription, value); }
    public string ReceiveFolder { get => _receiveFolder; private set => SetProperty(ref _receiveFolder, value); }
    public bool TrayIcon { get => _trayIcon; set => SetProperty(ref _trayIcon, value); }
    public bool StartAtSignIn { get => _startAtSignIn; private set => SetProperty(ref _startAtSignIn, value); }
    public bool ExplorerIntegration { get => _explorerIntegration; private set => SetProperty(ref _explorerIntegration, value); }
    public bool HasNotifications { get => _hasNotifications; private set => SetProperty(ref _hasNotifications, value); }
    public bool HasLinks { get => _hasLinks; private set => SetProperty(ref _hasLinks, value); }
    public string DataDirectory => _dataDir;
    public string? StartupSendError { get; }

    private string DaemonPath => Path.Combine(AppContext.BaseDirectory, "conduit-daemon.exe");

    public MainViewModel()
    {
        _dataDir = ResolveDataDir();
        MigrateLegacyData(_dataDir);
        StartupSendError = ReadStartupSendError();
    }

    public void Initialize(DispatcherQueue dispatcher)
    {
        _dispatcher = dispatcher;
        Directory.CreateDirectory(_dataDir);
        EnsureDaemonRunning();
        RefreshAll();
        StartDataWatcher();
    }

    public void RefreshAll()
    {
        if (_loading) return;
        _loading = true;
        try
        {
            Directory.CreateDirectory(_dataDir);
            LoadStatusAndConfig();
            LoadWallpaper();
            LoadNotifications();
            LoadLinks();
        }
        finally
        {
            _loading = false;
        }
    }

    private void LoadStatusAndConfig()
    {
        var status = ReadPairs(Path.Combine(_dataDir, "status.txt"));
        var config = ReadPairs(Path.Combine(_dataDir, "config.txt"));
        var state = Get(status, "state");
        var peer = Get(status, "peer_name");
        var path = Get(status, "path");
        var relay = Get(status, "relay");
        var linked = state.Equals("linked", StringComparison.OrdinalIgnoreCase);

        DeviceName = string.IsNullOrWhiteSpace(peer) ? "No phone" : peer;
        ConnectionState = PrettyState(state);
        IsLinked = linked;
        ConnectionGlyph = linked ? "\uE73E" : "\uE711";
        ConnectionRoute = linked
            ? PrettyRoute(path, relay)
            : (string.IsNullOrWhiteSpace(path) ? "—" : path);

        LoadRelaySelection(Get(config, "relays"));
        LoadProxySelection(Get(config, "relay_proxy"));
        SystemProxyDescription = ReadSystemProxyDescription();
        _receiveDir = Get(config, "receive_dir").Trim();
        if (string.IsNullOrWhiteSpace(_receiveDir)) _receiveDir = null;
        ReceiveFolder = _receiveDir ?? DownloadsFolder();
        TrayIcon = Get(config, "tray_icon").Equals("true", StringComparison.OrdinalIgnoreCase);
        StartAtSignIn = Registry.CurrentUser
            .OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Run")
            ?.GetValue("Conduit") is not null;
        ExplorerIntegration = Registry.CurrentUser
            .OpenSubKey(@"Software\Classes\*\shell\Conduit.SendToPhone") is not null;
    }

    private void LoadWallpaper()
    {
        var file = Path.Combine(_dataDir, "wallpaper.jpg");
        if (!File.Exists(file))
        {
            WallpaperSource = null;
            return;
        }
        try
        {
            // A fresh BitmapImage instance on every real file-system change keeps the UI cache
            // coherent while the daemon continues to use one bounded file on disk.
            WallpaperSource = new BitmapImage(new Uri(file));
        }
        catch
        {
            WallpaperSource = null;
        }
    }

    private void LoadNotifications()
    {
        try
        {
            var next = new List<NotificationEntry>();
            var file = Path.Combine(_dataDir, "notifications.tsv");
            if (File.Exists(file))
            {
                foreach (var line in File.ReadLines(file).Take(100))
                {
                    var parts = line.Split('\t', 6);
                    if (parts.Length < 2 || !long.TryParse(parts[0], out var timestamp)) continue;
                    var package = parts.Length > 2 ? parts[2].Trim() : string.Empty;
                    var app = parts.Length > 3 ? parts[3].Trim() : string.Empty;
                    var title = parts.Length > 4 ? parts[4].Trim() : string.Empty;
                    var body = parts.Length > 5 ? parts[5].Trim() : string.Empty;
                    var displayApp = !string.IsNullOrWhiteSpace(app)
                        ? app
                        : (!string.IsNullOrWhiteSpace(package) ? package : "Phone");
                    var iconPath = NotificationIconPath(package);
                    ImageSource? icon = null;
                    if (iconPath is not null)
                    {
                        try { icon = new BitmapImage(new Uri(iconPath)); } catch { }
                    }
                    next.Add(new NotificationEntry(
                        timestamp,
                        displayApp,
                        string.IsNullOrWhiteSpace(title) ? "Notification" : title,
                        body,
                        AgeLabel(timestamp),
                        icon));
                }
            }

            Notifications.Clear();
            foreach (var item in next) Notifications.Add(item);
            HasNotifications = Notifications.Count > 0;
        }
        catch (IOException)
        {
            // The writer replaces this bounded TSV atomically enough that the next file-system
            // notification will retry. Never add a polling loop just to cover a transient read.
        }
    }

    private void LoadLinks()
    {
        try
        {
            var next = new List<SharedLinkEntry>();
            var file = Path.Combine(_dataDir, "shared-links.tsv");
            if (File.Exists(file))
            {
                foreach (var line in File.ReadLines(file).Take(200))
                {
                    var parts = line.Split('\t');
                    if (parts.Length < 2 || !long.TryParse(parts[0], out var timestamp)) continue;
                    var url = parts[1];
                    if (!Uri.TryCreate(url, UriKind.Absolute, out var uri) ||
                        (uri.Scheme != Uri.UriSchemeHttp && uri.Scheme != Uri.UriSchemeHttps)) continue;
                    var title = parts.Length > 2 ? parts[2].Trim() : string.Empty;
                    var source = parts.Length > 3 ? parts[3].Trim() : string.Empty;
                    next.Add(new SharedLinkEntry(
                        timestamp,
                        url,
                        string.IsNullOrWhiteSpace(title) ? url : title,
                        $"{AgeLabel(timestamp)}  ·  {(string.IsNullOrWhiteSpace(source) ? "Phone" : source)}"));
                }
            }

            SharedLinks.Clear();
            foreach (var item in next) SharedLinks.Add(item);
            HasLinks = SharedLinks.Count > 0;
        }
        catch (IOException)
        {
            // Event-driven retry on the next real write; no timer.
        }
    }

    private void StartDataWatcher()
    {
        if (_dataWatcher is not null) return;
        var watcher = new FileSystemWatcher(_dataDir)
        {
            Filter = "*.*",
            NotifyFilter = NotifyFilters.LastWrite | NotifyFilters.FileName | NotifyFilters.Size,
            IncludeSubdirectories = false,
            EnableRaisingEvents = true,
        };
        watcher.Changed += DataChanged;
        watcher.Created += DataChanged;
        watcher.Deleted += DataChanged;
        watcher.Renamed += DataChanged;
        _dataWatcher = watcher;
    }

    private void DataChanged(object sender, FileSystemEventArgs e)
    {
        var name = Path.GetFileName(e.FullPath);
        if (!name.Equals("status.txt", StringComparison.OrdinalIgnoreCase) &&
            !name.Equals("config.txt", StringComparison.OrdinalIgnoreCase) &&
            !name.Equals("notifications.tsv", StringComparison.OrdinalIgnoreCase) &&
            !name.Equals("shared-links.tsv", StringComparison.OrdinalIgnoreCase) &&
            !name.Equals("wallpaper.jpg", StringComparison.OrdinalIgnoreCase)) return;

        _dispatcher?.TryEnqueue(() =>
        {
            if (name.Equals("notifications.tsv", StringComparison.OrdinalIgnoreCase)) LoadNotifications();
            else if (name.Equals("shared-links.tsv", StringComparison.OrdinalIgnoreCase)) LoadLinks();
            else if (name.Equals("wallpaper.jpg", StringComparison.OrdinalIgnoreCase)) LoadWallpaper();
            else LoadStatusAndConfig();
        });
    }

    public bool SaveSettings()
    {
        Directory.CreateDirectory(_dataDir);
        var configPath = Path.Combine(_dataDir, "config.txt");
        var config = ReadPairs(configPath);
        config["relays"] = string.Join(';', EnabledRelayEndpoints());
        config["relay_proxy"] = ProxyMode switch
        {
            "System proxy" => "system",
            "Manual SOCKS5" => ManualProxy.Replace("\r", "").Replace("\n", "").Trim(),
            _ => string.Empty,
        };
        config["tray_icon"] = TrayIcon ? "true" : "false";
        config["receive_dir"] = _receiveDir ?? string.Empty;

        var preferred = new[] { "relays", "relay_proxy", "tray_icon", "receive_dir" };
        var lines = new List<string>();
        foreach (var key in preferred)
            if (config.Remove(key, out var value)) lines.Add($"{key}={value}");
        lines.AddRange(config.Select(pair => $"{pair.Key}={pair.Value}"));
        File.WriteAllText(configPath, string.Join('\n', lines) + "\n");
        var applied = RunDaemonCommand("reload");
        RefreshAll();
        return applied;
    }

    private int EnabledRelayCount =>
        (RelayUs ? 1 : 0) + (RelayWa ? 1 : 0) + (RelayTyo ? 1 : 0) + (RelayJp ? 1 : 0);

    private void SetRelay(ref bool field, bool value)
    {
        if (!SetProperty(ref field, value)) return;
        OnPropertyChanged(nameof(RelaySummary));
    }

    private void LoadRelaySelection(string saved)
    {
        var endpoints = saved
            .Split(['\r', '\n', ';', ','], StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
            .ToArray();
        if (endpoints.Length == 0)
        {
            RelayUs = RelayWa = RelayTyo = RelayJp = true;
            return;
        }

        RelayUs = ContainsRelay(endpoints, "conduit-us.414222.xyz:41113", "us.414222.xyz:41113");
        RelayWa = ContainsRelay(endpoints, "conduit-wa.414222.xyz:41113", "wa.414222.xyz:41113");
        RelayTyo = ContainsRelay(endpoints, "conduit-tyo.414222.xyz:41113", "tyo.414222.xyz:41113");
        RelayJp = ContainsRelay(endpoints, "conduit-jp.414222.xyz:41113", "jp.414222.xyz:41113");

        // The old production fleet contained exactly US/TYO/WA. Treat that known three-node
        // inventory as a product default and migrate it to the new four-node managed fleet rather
        // than making JP look like a mysterious opt-in created by the UI rewrite.
        var known = endpoints.All(endpoint => ContainsRelay(
            [endpoint],
            "conduit-us.414222.xyz:41113", "us.414222.xyz:41113",
            "conduit-wa.414222.xyz:41113", "wa.414222.xyz:41113",
            "conduit-tyo.414222.xyz:41113", "tyo.414222.xyz:41113",
            "conduit-jp.414222.xyz:41113", "jp.414222.xyz:41113"));
        if (known && RelayUs && RelayWa && RelayTyo && !RelayJp)
            RelayJp = true;
    }

    private static bool ContainsRelay(IEnumerable<string> values, params string[] candidates) =>
        values.Any(value => candidates.Any(candidate => value.Equals(candidate, StringComparison.OrdinalIgnoreCase)));

    private IEnumerable<string> EnabledRelayEndpoints()
    {
        if (RelayUs) yield return "conduit-us.414222.xyz:41113";
        if (RelayWa) yield return "conduit-wa.414222.xyz:41113";
        if (RelayTyo) yield return "conduit-tyo.414222.xyz:41113";
        if (RelayJp) yield return "conduit-jp.414222.xyz:41113";
    }

    private void LoadProxySelection(string saved)
    {
        var value = saved.Trim();
        if (value.Equals("system", StringComparison.OrdinalIgnoreCase))
        {
            ProxyMode = "System proxy";
            ManualProxy = string.Empty;
        }
        else if (string.IsNullOrWhiteSpace(value))
        {
            ProxyMode = "Direct";
            ManualProxy = string.Empty;
        }
        else
        {
            ProxyMode = "Manual SOCKS5";
            ManualProxy = value;
        }
    }

    private static string ReadSystemProxyDescription()
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Internet Settings");
            var enabled = Convert.ToInt32(key?.GetValue("ProxyEnable") ?? 0) != 0;
            var value = key?.GetValue("ProxyServer")?.ToString()?.Trim();
            if (!enabled || string.IsNullOrWhiteSpace(value)) return "Windows system proxy is off";
            var endpoint = SystemSocksEndpoint(value);
            return endpoint is null
                ? $"Windows proxy: {value} (no SOCKS endpoint)"
                : $"Windows SOCKS: {endpoint}";
        }
        catch
        {
            return "Windows system proxy is unavailable";
        }
    }

    private static string? SystemSocksEndpoint(string value)
    {
        if (!value.Contains('=')) return value;
        foreach (var part in value.Split(';', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
        {
            var split = part.IndexOf('=');
            if (split <= 0) continue;
            var scheme = part[..split].Trim();
            if (scheme.Equals("socks", StringComparison.OrdinalIgnoreCase) ||
                scheme.Equals("socks5", StringComparison.OrdinalIgnoreCase))
                return part[(split + 1)..].Trim();
        }
        return null;
    }

    public bool SetAutostart(bool enabled)
    {
        var ok = RunDaemonCommand("autostart", enabled ? "install" : "remove");
        if (ok) StartAtSignIn = enabled; else LoadStatusAndConfig();
        return ok;
    }

    public bool SetExplorerIntegration(bool enabled)
    {
        var ok = RunDaemonCommand("explorer", enabled ? "install" : "remove");
        if (ok) ExplorerIntegration = enabled; else LoadStatusAndConfig();
        return ok;
    }

    public void SetReceiveFolder(string? folder)
    {
        _receiveDir = string.IsNullOrWhiteSpace(folder) ? null : Path.GetFullPath(folder);
        ReceiveFolder = _receiveDir ?? DownloadsFolder();
    }

    public void ClearNotifications()
    {
        var file = Path.Combine(_dataDir, "notifications.tsv");
        if (File.Exists(file)) File.Delete(file);
        LoadNotifications();
    }

    public void ClearLinks()
    {
        var file = Path.Combine(_dataDir, "shared-links.tsv");
        if (File.Exists(file)) File.Delete(file);
        LoadLinks();
    }

    public async Task<(bool Success, string Detail)> SendFileAsync(string path) =>
        await Task.Run(() => RunDaemonCommandDetailed("send", path));

    public string Diagnostics() => RunDaemonStatus();

    public void OpenDataFolder()
    {
        Directory.CreateDirectory(_dataDir);
        Process.Start(new ProcessStartInfo("explorer.exe", _dataDir) { UseShellExecute = true });
    }

    private void EnsureDaemonRunning()
    {
        if (Process.GetProcessesByName("conduit-daemon").Length > 0 || !File.Exists(DaemonPath)) return;
        try
        {
            Process.Start(new ProcessStartInfo(DaemonPath)
            {
                UseShellExecute = false,
                CreateNoWindow = true,
                WindowStyle = ProcessWindowStyle.Hidden,
                WorkingDirectory = AppContext.BaseDirectory,
            });
        }
        catch { }
    }

    private bool RunDaemonCommand(params string[] args) => RunDaemonCommandDetailed(args).Success;

    private (bool Success, string Detail) RunDaemonCommandDetailed(params string[] args)
    {
        if (!File.Exists(DaemonPath)) return (false, "conduit-daemon.exe was not found beside Conduit.exe.");
        try
        {
            var info = new ProcessStartInfo(DaemonPath)
            {
                UseShellExecute = false,
                CreateNoWindow = true,
                WindowStyle = ProcessWindowStyle.Hidden,
                WorkingDirectory = AppContext.BaseDirectory,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
            };
            foreach (var arg in args) info.ArgumentList.Add(arg);
            using var process = Process.Start(info);
            if (process is null) return (false, "Could not start conduit-daemon.exe.");
            var output = process.StandardOutput.ReadToEnd();
            var error = process.StandardError.ReadToEnd();
            process.WaitForExit();
            var detail = string.IsNullOrWhiteSpace(error) ? output.Trim() : error.Trim();
            if (detail.StartsWith("Error: ", StringComparison.OrdinalIgnoreCase)) detail = detail[7..].Trim();
            if (string.IsNullOrWhiteSpace(detail) && process.ExitCode != 0) detail = $"Command exited with code {process.ExitCode}.";
            return (process.ExitCode == 0, detail);
        }
        catch (Exception ex)
        {
            return (false, ex.Message);
        }
    }

    private string RunDaemonStatus()
    {
        var result = RunDaemonCommandDetailed("status");
        return string.IsNullOrWhiteSpace(result.Detail)
            ? (result.Success ? "Conduit daemon is running." : "Conduit daemon status is unavailable.")
            : result.Detail;
    }

    private string? NotificationIconPath(string package)
    {
        if (string.IsNullOrWhiteSpace(package)) return null;
        var hash = SHA256.HashData(Encoding.UTF8.GetBytes(package));
        var name = Convert.ToHexString(hash.AsSpan(0, 8)).ToLowerInvariant() + ".png";
        var path = Path.Combine(_dataDir, "icons", name);
        return File.Exists(path) ? path : null;
    }

    private static string ResolveDataDir()
    {
        var overridden = Environment.GetEnvironmentVariable("CONDUIT_DATA_DIR");
        return !string.IsNullOrWhiteSpace(overridden)
            ? Path.GetFullPath(overridden)
            : Path.Combine(AppContext.BaseDirectory, "data");
    }

    private static void MigrateLegacyData(string destination)
    {
        if (Directory.Exists(destination)) return;
        var legacy = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Conduit");
        if (!Directory.Exists(legacy) || Path.GetFullPath(legacy).Equals(Path.GetFullPath(destination), StringComparison.OrdinalIgnoreCase)) return;
        try
        {
            Directory.CreateDirectory(destination);
            foreach (var directory in Directory.EnumerateDirectories(legacy, "*", SearchOption.AllDirectories))
                Directory.CreateDirectory(Path.Combine(destination, Path.GetRelativePath(legacy, directory)));
            foreach (var file in Directory.EnumerateFiles(legacy, "*", SearchOption.AllDirectories))
            {
                var target = Path.Combine(destination, Path.GetRelativePath(legacy, file));
                if (!File.Exists(target)) File.Copy(file, target);
            }
        }
        catch { }
    }

    private static Dictionary<string, string> ReadPairs(string path)
    {
        var result = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        if (!File.Exists(path)) return result;
        foreach (var line in File.ReadLines(path))
        {
            var split = line.IndexOf('=');
            if (split <= 0) continue;
            result[line[..split].Trim()] = line[(split + 1)..].Trim();
        }
        return result;
    }

    private static string Get(Dictionary<string, string> values, string key) =>
        values.TryGetValue(key, out var value) ? value : string.Empty;

    private static string PrettyState(string state)
    {
        if (string.IsNullOrWhiteSpace(state)) return "Disconnected";
        return state.ToLowerInvariant() switch
        {
            "linked" => "Connected",
            "connecting" => "Connecting",
            "retrying" => "Reconnecting",
            "running" => "Running",
            _ => char.ToUpperInvariant(state[0]) + state[1..],
        };
    }

    private static string PrettyRoute(string path, string relay)
    {
        var route = string.IsNullOrWhiteSpace(path) ? string.Empty : char.ToUpperInvariant(path[0]) + path[1..].ToLowerInvariant();
        if (!route.Equals("Relay", StringComparison.OrdinalIgnoreCase) || string.IsNullOrWhiteSpace(relay))
            return string.IsNullOrWhiteSpace(route) ? "Connected" : route;
        var host = relay.Split(':', 2)[0];
        var site = host.Split('.', StringSplitOptions.RemoveEmptyEntries).FirstOrDefault() ?? host;
        return $"Relay  ·  {site.ToUpperInvariant()}";
    }

    private static string AgeLabel(long timestampMs)
    {
        var delta = Math.Max(0, DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() - timestampMs);
        if (delta < 60_000) return "now";
        if (delta < 3_600_000) return $"{delta / 60_000}m";
        if (delta < 86_400_000) return $"{delta / 3_600_000}h";
        return $"{delta / 86_400_000}d";
    }

    private static string DownloadsFolder()
    {
        const string downloadsId = "{374DE290-123F-4565-9164-39C4925E467B}";
        using var key = Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders");
        var configured = key?.GetValue(downloadsId, null, RegistryValueOptions.DoNotExpandEnvironmentNames) as string;
        if (!string.IsNullOrWhiteSpace(configured)) return Environment.ExpandEnvironmentVariables(configured);
        return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), "Downloads");
    }

    private static string? ReadStartupSendError()
    {
        var args = Environment.GetCommandLineArgs();
        for (var i = 1; i + 1 < args.Length; i++)
            if (args[i].Equals("--send-error", StringComparison.OrdinalIgnoreCase)) return args[i + 1];
        return null;
    }

    public void Dispose()
    {
        _dataWatcher?.Dispose();
        _dataWatcher = null;
    }
}
