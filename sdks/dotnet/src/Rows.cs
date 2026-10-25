namespace Hairspring;

/// <summary>The immutable values the protocol lines are made from. Nothing here reads the process.</summary>
internal static class Rows
{
    /// <summary>What the lines say about the process.</summary>
    public sealed record Proc(string Tag, bool TagStable, bool SignalHandlers, ulong CatalogHash, int NIds, ulong Dropped);

    public sealed record CatalogRow(string Id, Kind Kind, string Msg, string File, long Line);

    /// <summary>A witness ready to print: the thread's key, that thread's own ordinal and four values.</summary>
    public sealed record Witness(string Key, ulong Ord, long[] Vals);

    public sealed record AssertRow(string Id, Kind Kind, ulong Trues, ulong Falses, Witness? FirstTrue, Witness? FirstFalse);

    public sealed record ThreadRow(string Key, ulong Evals, ulong TapeConsumed, bool Stable);
}
