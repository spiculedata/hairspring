using System;

namespace LangAt;

/// <summary>The FNV-1a 64 hash of the arrival list written as ASCII digits: the <c>order</c> field of the result line.</summary>
internal static class OrderHash
{
    private const ulong Offset = 0xcbf29ce484222325UL;
    private const ulong Prime = 0x00000100000001b3UL;

    public static ulong Fnv1a64(ReadOnlySpan<byte> bytes)
    {
        var hash = Offset;
        foreach (var one in bytes)
        {
            hash = unchecked((hash ^ one) * Prime);
        }
        return hash;
    }

    public static ulong Of(int[] arrival)
    {
        var digits = new byte[arrival.Length];
        for (var i = 0; i < digits.Length; i++)
        {
            digits[i] = (byte)('0' + (arrival[i] % 10));
        }
        return Fnv1a64(digits);
    }
}
