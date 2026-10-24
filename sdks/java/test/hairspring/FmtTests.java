package hairspring;

/** The formatter: widths and padding match the C SDK, so a line's length never depends on a counter's value. */
final class FmtTests {
    private FmtTests() {}

    static void register(Suite s) {
        s.add("fmt: counters are right-aligned in twenty characters", () -> {
            Check.equal(" ".repeat(19) + "5", Fmt.numW(5));
            Check.equal("18446744073709551615", Fmt.numW(-1L));
            Check.equal(20, Fmt.numW(0).length());
        });
        s.add("fmt: signed counters keep the sign inside the field", () -> {
            Check.equal(" ".repeat(18) + "-7", Fmt.i64W(-7));
            Check.equal(20, Fmt.i64W(Long.MIN_VALUE).length());
            Check.yes(Fmt.i64W(Long.MIN_VALUE).endsWith("-9223372036854775808"), "min value");
            Check.equal(" ".repeat(18) + "12", Fmt.i64W(12));
        });
        s.add("fmt: hex is sixteen lowercase digits", () -> {
            Check.equal("fb6cf21a75aad6ef", Fmt.hex64(0xFB6CF21A75AAD6EFL));
            Check.equal("00000000000000ab", Fmt.hex64(0xABL));
        });
        s.add("fmt: free text escapes quotes and backslashes and masks other bytes", () -> {
            // U+00E9 is two bytes in UTF-8, so it becomes two question marks; a newline is one.
            Check.equal("\"a\\\"b\\\\c???\"", Fmt.jsonText("a\"b\\cé\n", Fmt.MSG_MAX));
        });
        s.add("fmt: free text is cut at the limit in bytes", () -> {
            Check.equal("\"abc\"", Fmt.jsonText("abcdef", 3));
            Check.equal("\"???\"", Fmt.jsonText("éé", 3));
        });
        s.add("fmt: free text stops at a NUL", () -> Check.equal("\"ab\"", Fmt.jsonText("ab\0cd", Fmt.MSG_MAX)));
        s.add("fmt: ids replace bytes outside the alphabet", () ->
                Check.equal("\"a_b_c.d:e/f-g_h\"", Fmt.ident("a b#c.d:e/f-g_h", Fmt.ID_MAX)));
        s.add("fmt: sanitize keeps the sent form and the limit", () -> {
            Check.equal("a_b", Fmt.sanitizeIdent("a b", Fmt.ID_MAX));
            Check.equal("abc", Fmt.sanitizeIdent("abcdef", 3));
            Check.equal("caf__", Fmt.sanitizeIdent("café", Fmt.ID_MAX));
        });
        s.add("fmt: thread keys keep the hash and pad to sixty four bytes", () -> {
            Check.equal("\"t#1\"" + " ".repeat(61), Fmt.threadKeyPadded("t#1"));
            Check.equal("\"a_b#c\"", Fmt.threadKey("a b#c", Fmt.THREAD_KEY_MAX));
        });
        s.add("fmt: the identifier alphabet", () -> {
            for (char ch : "azAZ09._:/-".toCharArray()) {
                Check.yes(Fmt.isIdentChar(ch), "in the alphabet: " + ch);
            }
            for (char ch : "# $\"\\".toCharArray()) {
                Check.no(Fmt.isIdentChar(ch), "in the alphabet: " + ch);
            }
        });
        s.add("fmt: the id hash uses the sent form and the limit", () -> {
            Check.equal(Fmt.identHash("a_b", Fmt.ID_MAX), Fmt.identHash("a b", Fmt.ID_MAX));
            Check.no(Fmt.identHash("a", Fmt.ID_MAX) == Fmt.identHash("ab", Fmt.ID_MAX), "a and ab differ");
            Check.equal(Fmt.identHash("abcY", 3), Fmt.identHash("abcX", 3));
        });
        s.add("fmt: the hash is FNV-1a 64", () -> {
            long h = 0xCBF29CE484222325L;
            for (byte c : "basic.always.ok".getBytes(java.nio.charset.StandardCharsets.US_ASCII)) {
                h = (h ^ (c & 0xffL)) * 0x00000100000001B3L;
            }
            Check.equal(h, Fmt.identHash("basic.always.ok", Fmt.ID_MAX));
            Check.equal(0xCBF29CE484222325L, Fmt.fnv1a64(new byte[0]));
        });
    }
}
