using System.Collections.Generic;

namespace Hairspring;

/// <summary>
/// The catalog's summary values: how many distinct ids, and the order-independent hash of the distinct (id, kind) pairs that
/// <c>hello.catalog_hash</c> carries. The hash equals the C SDK's for the same ids and kinds.
/// </summary>
internal static class Catalog
{
    private const int KindShift = 56;
    private const ulong HashMix = 0x9E3779B97F4A7C15UL;

    /// <summary>One distinct (id, kind) pair.</summary>
    public readonly record struct Entry(string Id, Kind Kind);

    public static ulong PairHash(Entry entry)
    {
        var mixed = Fmt.IdentHash(entry.Id, Fmt.IdMax) ^ ((ulong)KindInfo.Number(entry.Kind) << KindShift);
        return unchecked(mixed * HashMix);
    }

    /// <summary>The sum of the pair hashes, so the order of the entries does not matter. Entries must be distinct pairs.</summary>
    public static ulong Hash(IEnumerable<Entry> entries)
    {
        ulong sum = 0;
        foreach (var entry in entries)
        {
            sum = unchecked(sum + PairHash(entry));
        }
        return sum;
    }

    public static int DistinctIds(IEnumerable<Entry> entries)
    {
        var ids = new HashSet<string>(System.StringComparer.Ordinal);
        foreach (var entry in entries)
        {
            ids.Add(entry.Id);
        }
        return ids.Count;
    }
}
