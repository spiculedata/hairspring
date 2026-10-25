namespace Hairspring;

/// <summary>
/// One (id, kind) pair and its counters. <c>Id</c> is the form that is sent. The identity fields never change; the counters
/// and witnesses are changed only with the recorder's lock held.
/// </summary>
internal sealed class Site
{
    /// <summary>Stands in the lookup for an id the table had no room for, so the hot path recognises it without registering again.</summary>
    public static readonly Site Untracked = new("", Kind.Reachable, "", "", 0);

    public Site(string id, Kind kind, string msg, string file, long line)
    {
        Id = id;
        Kind = kind;
        Point = KindInfo.IsPoint(kind);
        Msg = msg;
        File = file;
        Line = line;
    }

    /// <summary>The first evaluation on one side of a site: which slot, that thread's ordinal and the four values.</summary>
    public sealed record Mark(int Slot, ulong Ord, long[] Vals);

    public string Id { get; }

    public Kind Kind { get; }

    public bool Point { get; }

    public string Msg { get; }

    public string File { get; }

    public long Line { get; }

    public ulong Trues { get; set; }

    public ulong Falses { get; set; }

    public Mark? FirstTrue { get; set; }

    public Mark? FirstFalse { get; set; }
}
