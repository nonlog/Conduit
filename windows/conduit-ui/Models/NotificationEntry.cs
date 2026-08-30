namespace Conduit.Models;

public sealed record NotificationEntry(
    long TimestampMs,
    string App,
    string Title,
    string Body,
    string Age,
    ImageSource? IconSource);
