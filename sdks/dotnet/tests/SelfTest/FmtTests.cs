using System.Text;

namespace Hairspring;

/// <summary>The formatter: widths and padding match the C SDK, so a line's length never depends on a counter's value.</summary>
internal static class FmtTests
{
    public static void Register(Suite s)
    {
        s.Add("fmt: counters are right-aligned in twenty characters", () =>
        {
            Check.Equal(new string(' ', 19) + "5", Fmt.NumW(5));
            Check.Equal("18446744073709551615", Fmt.NumW(ulong.MaxValue));
            Check.Equal(20, Fmt.NumW(0).Length);
        });
        s.Add("fmt: signed counters keep the sign inside the field", () =>
        {
            Check.Equal(new string(' ', 18) + "-7", Fmt.I64W(-7));
            Check.Equal(20, Fmt.I64W(long.MinValue).Length);
            Check.Yes(Fmt.I64W(long.MinValue).EndsWith("-9223372036854775808", System.StringComparison.Ordinal), "min value");
            Check.Equal(new string(' ', 18) + "12", Fmt.I64W(12));
        });
        s.Add("fmt: numbers do not depend on the culture", () =>
        {
            var saved = System.Globalization.CultureInfo.CurrentCulture;
            try
            {
                System.Globalization.CultureInfo.CurrentCulture = new System.Globalization.CultureInfo("ar-SA");
                Check.Equal(new string(' ', 19) + "5", Fmt.NumW(5));
                Check.Equal(new string(' ', 18) + "-7", Fmt.I64W(-7));
                Check.Equal("fb6cf21a75aad6ef", Fmt.Hex64(0xFB6CF21A75AAD6EFUL));
            }
            finally
            {
                System.Globalization.CultureInfo.CurrentCulture = saved;
            }
        });
        s.Add("fmt: hex is sixteen lowercase digits", () =>
        {
            Check.Equal("fb6cf21a75aad6ef", Fmt.Hex64(0xFB6CF21A75AAD6EFUL));
            Check.Equal("00000000000000ab", Fmt.Hex64(0xAB));
        });
        s.Add("fmt: free text escapes quotes and backslashes and masks other bytes", () =>
        {
            // U+00E9 is two bytes in UTF-8, so it becomes two question marks; a newline is one.
            Check.Equal("\"a\\\"b\\\\c???\"", Fmt.JsonText("a\"b\\c\u00e9\n", Fmt.MsgMax));
        });
        s.Add("fmt: control, format and bidirectional characters in text become question marks", () =>
        {
            // U+202E (right-to-left override), U+200B (zero width space), U+2028 (line separator), U+FEFF, U+0085, U+007F, U+0000..
            foreach (var text in new[] { "\u200b", "\u2028", "\ufeff", "\u0085", "\u007f", "\u0001", "\u202e", "\u2066" })
            {
                var json = Fmt.JsonText("x" + text + "y", Fmt.MsgMax);
                foreach (var ch in json)
                {
                    Check.Yes(ch >= 0x20 && ch <= 0x7e, $"only printable ASCII is sent, got U+{(int)ch:X4} from U+{(int)text[0]:X4}");
                }
                Check.Yes(json.Contains('?', System.StringComparison.Ordinal), "a masked byte");
            }
        });
        s.Add("fmt: free text is cut at the limit in bytes", () =>
        {
            Check.Equal("\"abc\"", Fmt.JsonText("abcdef", 3));
            Check.Equal("\"???\"", Fmt.JsonText("\u00e9\u00e9", 3));
        });
        s.Add("fmt: a NUL in free text is sent as a question mark, not as the end of the text", () => Check.Equal("\"ab?cd\"", Fmt.JsonText("ab\0cd", Fmt.MsgMax)));
        s.Add("fmt: a lone surrogate is masked, not an error", () =>
        {
            var json = Fmt.JsonText("a\ud800b", Fmt.MsgMax);
            Check.Yes(json.StartsWith("\"a?", System.StringComparison.Ordinal) && json.EndsWith("b\"", System.StringComparison.Ordinal), json);
        });
        s.Add("fmt: ids replace bytes outside the alphabet", () =>
            Check.Equal("\"a_b_c.d:e/f-g_h\"", Fmt.Ident("a b#c.d:e/f-g_h", Fmt.IdMax)));
        s.Add("fmt: sanitize keeps the sent form and the limit", () =>
        {
            Check.Equal("a_b", Fmt.SanitizeIdent("a b", Fmt.IdMax));
            Check.Equal("abc", Fmt.SanitizeIdent("abcdef", 3));
            Check.Equal("caf__", Fmt.SanitizeIdent("caf\u00e9", Fmt.IdMax));
        });
        s.Add("fmt: thread keys keep the hash and pad to sixty four bytes", () =>
        {
            Check.Equal("\"t#1\"" + new string(' ', 61), Fmt.ThreadKeyPadded("t#1"));
            Check.Equal("\"a_b#c\"", Fmt.ThreadKey("a b#c", Fmt.ThreadKeyMax));
        });
        s.Add("fmt: the identifier alphabet", () =>
        {
            foreach (var ch in "azAZ09._:/-")
            {
                Check.Yes(Fmt.IsIdentChar(ch), $"in the alphabet: {ch}");
            }
            foreach (var ch in "# $\"\\")
            {
                Check.No(Fmt.IsIdentChar(ch), $"in the alphabet: {ch}");
            }
        });
        s.Add("fmt: the id hash uses the sent form and the limit", () =>
        {
            Check.Equal(Fmt.IdentHash("a_b", Fmt.IdMax), Fmt.IdentHash("a b", Fmt.IdMax));
            Check.No(Fmt.IdentHash("a", Fmt.IdMax) == Fmt.IdentHash("ab", Fmt.IdMax), "a and ab differ");
            Check.Equal(Fmt.IdentHash("abcY", 3), Fmt.IdentHash("abcX", 3));
        });
        s.Add("fmt: the hash is FNV-1a 64", () =>
        {
            var h = 0xCBF29CE484222325UL;
            foreach (var c in Encoding.ASCII.GetBytes("basic.always.ok"))
            {
                h = unchecked((h ^ c) * 0x00000100000001B3UL);
            }
            Check.Equal(h, Fmt.IdentHash("basic.always.ok", Fmt.IdMax));
            Check.Equal(0xCBF29CE484222325UL, Fmt.Fnv1a64(System.Array.Empty<byte>()));
        });
    }
}
