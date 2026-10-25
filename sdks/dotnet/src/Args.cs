using System;

namespace Hairspring;

/// <summary>Validation of what a call site passes. Called before any state changes, so a bad argument counts nothing.</summary>
internal static class Args
{
    public const int MaxVals = 4;

    /// <summary>A UTF-16 unit is at most three bytes of UTF-8 (a surrogate pair is four bytes for two units).</summary>
    private const int MaxUtf8BytesPerUnit = 3;

    public static void CheckId(string raw)
    {
        ArgumentNullException.ThrowIfNull(raw, "id");
        var size = System.Text.Encoding.UTF8.GetByteCount(raw);
        if (size < 1 || size > Fmt.IdMax)
        {
            throw new ArgumentException($"an assertion id is 1 to {Fmt.IdMax} bytes, not {size}", "id");
        }
        if (raw.Contains('\0', StringComparison.Ordinal))
        {
            // The wire form stops at a NUL, so the id would be sent shortened or empty.
            throw new ArgumentException("an assertion id must not contain a NUL character", "id");
        }
    }

    public static void CheckMsg(string raw)
    {
        ArgumentNullException.ThrowIfNull(raw, "msg");
        if ((long)raw.Length * MaxUtf8BytesPerUnit <= Fmt.MsgMax)
        {
            // Short enough that it cannot be over the limit: no measuring. A longer message (over 170 characters) is
            // measured on every call, without allocating.
            return;
        }
        var size = System.Text.Encoding.UTF8.GetByteCount(raw);
        if (size > Fmt.MsgMax)
        {
            throw new ArgumentException($"an assertion message is at most {Fmt.MsgMax} bytes, not {size}", "msg");
        }
    }

    /// <summary>Fails on a fifth value; cheap enough for every call.</summary>
    public static void CheckVals(ReadOnlySpan<long> vals)
    {
        if (vals.Length > MaxVals)
        {
            throw new ArgumentException($"an assertion takes at most {MaxVals} values, not {vals.Length}", "vals");
        }
    }

    /// <summary>Up to four values, zero-filled.</summary>
    public static long[] PackVals(ReadOnlySpan<long> vals)
    {
        CheckVals(vals);
        var packed = new long[MaxVals];
        vals.CopyTo(packed);
        return packed;
    }
}
