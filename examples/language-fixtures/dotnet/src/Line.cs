using System;
using System.Collections.Generic;
using System.Globalization;

namespace LangAt;

/// <summary>The six values of the line, in the order they are printed.</summary>
internal readonly record struct Fields(ulong LockOrder, ulong Arrival, ulong Rand, ulong VtimeNs, ulong Extra);

/// <summary>The one stdout line and the hashes in it.</summary>
internal static class Line
{
    private const ulong FnvOffset = 0xcbf29ce484222325UL;
    private const ulong FnvPrime = 0x00000100000001b3UL;

    /// <summary>FNV-1a 64 of <paramref name="bytes"/>.</summary>
    public static ulong Fnv1a64(ReadOnlySpan<byte> bytes)
    {
        var hash = FnvOffset;
        foreach (var value in bytes)
        {
            hash = unchecked((hash ^ value) * FnvPrime);
        }
        return hash;
    }

    /// <summary>The hash of a thread order (worker numbers 0 to 3) written as bare ASCII digits, like the fixtures' order hash.</summary>
    public static ulong HashDigits(IReadOnlyList<int> items)
    {
        var digits = new byte[items.Count];
        for (var i = 0; i < digits.Length; i++)
        {
            digits[i] = (byte)('0' + (items[i] % 10));
        }
        return Fnv1a64(digits);
    }

    /// <summary>
    /// <c>PROBE v1 lang=dotnet lock_order=&lt;16 hex&gt; arrival=&lt;16 hex&gt; rand=&lt;16 hex&gt; vtime_ns=&lt;20 digits&gt; extra=&lt;16 hex&gt;</c>
    /// and a newline, formatted the same under every culture.
    /// </summary>
    public static string Format(Fields fields)
    {
        var c = CultureInfo.InvariantCulture;
        return "PROBE v1 lang=dotnet" +
               " lock_order=" + fields.LockOrder.ToString("x16", c) +
               " arrival=" + fields.Arrival.ToString("x16", c) +
               " rand=" + fields.Rand.ToString("x16", c) +
               " vtime_ns=" + fields.VtimeNs.ToString("D20", c) +
               " extra=" + fields.Extra.ToString("x16", c) +
               "\n";
    }
}
