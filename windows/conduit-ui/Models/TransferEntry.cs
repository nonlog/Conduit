namespace Conduit.Models;

public sealed class TransferEntry : ObservableObject
{
    public string Id { get; }
    public string Name { get; private set; } = string.Empty;
    public string Direction { get; private set; } = string.Empty;
    public ulong Bytes { get; private set; }
    public ulong Total { get; private set; }
    public string State { get; private set; } = string.Empty;
    public string Detail { get; private set; } = string.Empty;
    private bool _cancelRequested;
    public double Percentage => Total == 0 ? 0 : Math.Clamp(100.0 * Bytes / Total, 0, 100);
    public bool IsTerminal => State is "completed" or "failed" or "cancelled";
    public bool CanCancel => !IsTerminal && !_cancelRequested;
    public Visibility CancelVisibility => IsTerminal ? Visibility.Collapsed : Visibility.Visible;
    public string CancelName => $"Cancel {Name}";
    public string Summary => $"{(Direction == "inbound" ? "Receiving" : "Sending")} · {Bytes:N0} / {Total:N0} B · {Percentage:0}% · {State}";
    public string Outcome => string.IsNullOrWhiteSpace(Detail) ? Summary : $"{Summary}\n{Detail}";

    private TransferEntry(string id) => Id = id;

    public static TransferEntry? Parse(string line)
    {
        var fields = line.Split('\t', 7);
        if (fields.Length != 7 || fields[0].Length is < 2 or > 128 || fields[0].Length % 2 != 0 ||
            !fields[0].All(char.IsAsciiHexDigit) || fields[1] is not ("inbound" or "outbound") ||
            !ulong.TryParse(fields[2], out var bytes) || !ulong.TryParse(fields[3], out var total) ||
            bytes > total || fields[4] is not ("queued" or "transferring" or "waiting confirmation" or "completed" or "failed" or "cancelled")) return null;
        return new TransferEntry(fields[0])
        {
            Direction = fields[1], Bytes = bytes, Total = total, State = fields[4], Name = fields[5], Detail = fields[6],
        };
    }

    public void Update(TransferEntry next)
    {
        if (Name == next.Name && Direction == next.Direction && Bytes == next.Bytes && Total == next.Total &&
            State == next.State && Detail == next.Detail) return;
        Name = next.Name; Direction = next.Direction; Bytes = next.Bytes; Total = next.Total;
        State = next.State; Detail = next.Detail;
        OnPropertyChanged(string.Empty);
    }

    public void CancellationRequested()
    {
        _cancelRequested = true;
        OnPropertyChanged(nameof(CanCancel));
    }
}
