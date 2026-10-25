using System.Collections.Generic;

namespace Hairspring;

/// <summary>An immutable snapshot of the recorder's state, taken with its lock held, so every count, witness and thread total in it belongs to one moment.</summary>
internal sealed record Snapshot(
    IReadOnlyList<Rows.CatalogRow> Catalog,
    IReadOnlyList<Rows.AssertRow> Asserts,
    IReadOnlyList<ThreadView> Threads,
    ulong CatalogHash,
    int NIds,
    ulong Dropped)
{
    /// <summary>The operations all threads counted (saturating).</summary>
    public ulong Work
    {
        get
        {
            ulong total = 0;
            foreach (var t in Threads)
            {
                total = Saturating.Add(total, t.Work);
            }
            return total;
        }
    }

    /// <summary>The evaluations all threads counted (saturating).</summary>
    public ulong Evals
    {
        get
        {
            ulong total = 0;
            foreach (var t in Threads)
            {
                total = Saturating.Add(total, t.Evals);
            }
            return total;
        }
    }
}

/// <summary>Counter arithmetic that stops at the largest value instead of wrapping.</summary>
internal static class Saturating
{
    public static ulong Add(ulong a, ulong b) => ulong.MaxValue - a < b ? ulong.MaxValue : a + b;
}
