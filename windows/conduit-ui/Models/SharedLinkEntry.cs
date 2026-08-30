namespace Conduit.Models;

public sealed record SharedLinkEntry(
    long TimestampMs,
    string Url,
    string Title,
    string Meta);
