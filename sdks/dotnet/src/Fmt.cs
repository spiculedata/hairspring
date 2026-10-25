using System;
using System.Globalization;
using System.Text;

namespace Hairspring;

/// <summary>
/// Pure formatting for the guest protocol: the widths and padding match the C SDK, so a line's length never depends on a
/// counter's value. Everything returns ASCII text and is independent of the current culture. Ids, tags and thread keys are
/// cut and tested in UTF-8 bytes, as the wire format counts them.
/// </summary>
internal static class Fmt
{
    public const int LineMax = 4096;
    public const int NumWidth = 20;
    public const int IdMax = 128;
    public const int ThreadKeyMax = 64;
    public const int MsgMax = 512;
    public const int FileMax = 256;
    public const int TagMax = 64;

    /// <summary>The widest witness object: thread key 64 bytes, ord 20 digits, four 20-character values.</summary>
    public const int WitnessWidth = 197;

    private const ulong FnvOffset = 0xCBF29CE484222325UL;
    private const ulong FnvPrime = 0x00000100000001B3UL;
    private const char Replacement = '_';
    private const int HexWidth = 16;
    private const int FirstPrintable = 0x20;
    private const int LastPrintable = 0x7E;

    private static CultureInfo Invariant => CultureInfo.InvariantCulture;

    /// <summary>An unsigned counter right-aligned in 20 characters.</summary>
    public static string NumW(ulong value) => value.ToString(Invariant).PadLeft(NumWidth);

    /// <summary>A signed counter right-aligned in 20 characters, the sign inside the field.</summary>
    public static string I64W(long value) => value.ToString(Invariant).PadLeft(NumWidth);

    /// <summary>Sixteen lowercase hex digits.</summary>
    public static string Hex64(ulong value) => value.ToString("x16", Invariant);

    /// <summary>A signed number without padding (witness values).</summary>
    public static string Signed(long value) => value.ToString(Invariant);

    public static bool IsIdentChar(int b) =>
        (b >= 'a' && b <= 'z') || (b >= 'A' && b <= 'Z') || (b >= '0' && b <= '9') || b is '.' or '_' or ':' or '/' or '-';

    public static byte[] Utf8(string text) => Encoding.UTF8.GetBytes(text);

    private static int UntilNul(ReadOnlySpan<byte> data, int limit)
    {
        var end = Math.Min(data.Length, limit);
        var nul = data[..end].IndexOf((byte)0);
        return nul < 0 ? end : nul;
    }

    /// <summary>
    /// A quoted JSON string of at most <paramref name="maxBytes"/> source bytes. Printable ASCII survives, <c>"</c> and
    /// <c>\</c> are escaped, and every other byte (control characters, format and bidirectional characters, anything outside
    /// ASCII) becomes <c>?</c>, which keeps free text inside the protocol's string rules.
    /// </summary>
    public static string JsonText(ReadOnlySpan<byte> text, int maxBytes)
    {
        var end = UntilNul(text, maxBytes);
        var output = new StringBuilder(end + 2).Append('"');
        for (var i = 0; i < end; i++)
        {
            int b = text[i];
            if (b is '"' or '\\')
            {
                output.Append('\\').Append((char)b);
            }
            else
            {
                output.Append(b is >= FirstPrintable and <= LastPrintable ? (char)b : '?');
            }
        }
        return output.Append('"').ToString();
    }

    public static string JsonText(string text, int maxBytes) => JsonText(Utf8(text), maxBytes);

    private static string Sent(string text, int maxBytes, bool allowHash)
    {
        var data = Utf8(text);
        var end = UntilNul(data, maxBytes);
        var output = new StringBuilder(end);
        for (var i = 0; i < end; i++)
        {
            int b = data[i];
            output.Append(IsIdentChar(b) || (allowHash && b == '#') ? (char)b : Replacement);
        }
        return output.ToString();
    }

    /// <summary>The form of an id or tag that is sent: bytes outside <c>[A-Za-z0-9._:/-]</c> become <c>_</c>, at most <paramref name="maxBytes"/>.</summary>
    public static string SanitizeIdent(string text, int maxBytes) => Sent(text, maxBytes, false);

    public static string Ident(string text, int maxBytes) => '"' + SanitizeIdent(text, maxBytes) + '"';

    /// <summary>Like an id, but <c>#</c> is allowed (claim-order keys are <c>t#&lt;n&gt;</c>).</summary>
    public static string ThreadKey(string text, int maxBytes) => '"' + Sent(text, maxBytes, true) + '"';

    /// <summary>The quoted key followed by spaces up to 64 key bytes, so a line's length does not depend on the key.</summary>
    public static string ThreadKeyPadded(string text)
    {
        var quoted = ThreadKey(text, ThreadKeyMax);
        var used = quoted.Length - 2;
        return used >= ThreadKeyMax ? quoted : quoted + new string(' ', ThreadKeyMax - used);
    }

    public static ulong Fnv1a64(ReadOnlySpan<byte> data)
    {
        var value = FnvOffset;
        foreach (var b in data)
        {
            value = unchecked((value ^ b) * FnvPrime);
        }
        return value;
    }

    /// <summary>FNV-1a 64 over the form of an id that is sent.</summary>
    public static ulong IdentHash(string text, int maxBytes) => Fnv1a64(Encoding.ASCII.GetBytes(SanitizeIdent(text, maxBytes)));

    /// <summary>Sixteen lowercase hex digits are what <see cref="Hex64"/> writes; this is its width.</summary>
    public static int HexDigits => HexWidth;
}
