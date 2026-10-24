package hairspring;

import java.nio.charset.StandardCharsets;

/**
 * Pure formatting for the guest protocol: the widths and padding match the C SDK, so a line's length never depends on a
 * counter's value. Everything returns ASCII text. Ids, tags and thread keys are cut and tested in UTF-8 bytes, as the
 * wire format counts them.
 */
final class Fmt {
    static final int LINE_MAX = 4096;
    static final int NUM_WIDTH = 20;
    static final int ID_MAX = 128;
    static final int THREAD_KEY_MAX = 64;
    static final int MSG_MAX = 512;
    static final int FILE_MAX = 256;
    static final int TAG_MAX = 64;
    /** The widest witness object: thread key 64 bytes, ord 20 digits, four 20-character values. */
    static final int WITNESS_WIDTH = 197;

    private static final long FNV_OFFSET = 0xCBF29CE484222325L;
    private static final long FNV_PRIME = 0x00000100000001B3L;
    private static final char REPLACEMENT = '_';
    private static final int HEX_WIDTH = 16;
    private static final int FIRST_PRINTABLE = 0x20;
    private static final int LAST_PRINTABLE = 0x7E;

    private Fmt() {}

    private static String padLeft(String digits, int width) {
        return digits.length() >= width ? digits : " ".repeat(width - digits.length()) + digits;
    }

    /** An unsigned counter right-aligned in 20 characters. */
    static String numW(long value) {
        return padLeft(Long.toUnsignedString(value), NUM_WIDTH);
    }

    /** A signed counter right-aligned in 20 characters, the sign inside the field. */
    static String i64W(long value) {
        return padLeft(Long.toString(value), NUM_WIDTH);
    }

    /** Sixteen lowercase hex digits. */
    static String hex64(long value) {
        String digits = Long.toHexString(value);
        return "0".repeat(HEX_WIDTH - digits.length()) + digits;
    }

    static boolean isIdentChar(int b) {
        return (b >= 'a' && b <= 'z')
                || (b >= 'A' && b <= 'Z')
                || (b >= '0' && b <= '9')
                || b == '.'
                || b == '_'
                || b == ':'
                || b == '/'
                || b == '-';
    }

    static byte[] utf8(String text) {
        return text.getBytes(StandardCharsets.UTF_8);
    }

    private static int untilNul(byte[] data, int limit) {
        int end = Math.min(data.length, limit);
        for (int i = 0; i < end; i++) {
            if (data[i] == 0) {
                return i;
            }
        }
        return end;
    }

    /**
     * A quoted JSON string of at most {@code maxBytes} source bytes. Printable ASCII survives, {@code "} and {@code \}
     * are escaped, and every other byte becomes {@code ?}, which keeps free text inside the protocol's string rules.
     */
    static String jsonText(byte[] text, int maxBytes) {
        StringBuilder out = new StringBuilder("\"");
        int end = untilNul(text, maxBytes);
        for (int i = 0; i < end; i++) {
            int b = text[i] & 0xff;
            if (b == '"' || b == '\\') {
                out.append('\\').append((char) b);
            } else if (b >= FIRST_PRINTABLE && b <= LAST_PRINTABLE) {
                out.append((char) b);
            } else {
                out.append('?');
            }
        }
        return out.append('"').toString();
    }

    static String jsonText(String text, int maxBytes) {
        return jsonText(utf8(text), maxBytes);
    }

    private static String sent(String text, int maxBytes, boolean allowHash) {
        byte[] data = utf8(text);
        int end = untilNul(data, maxBytes);
        StringBuilder out = new StringBuilder(end);
        for (int i = 0; i < end; i++) {
            int b = data[i] & 0xff;
            out.append(isIdentChar(b) || (allowHash && b == '#') ? (char) b : REPLACEMENT);
        }
        return out.toString();
    }

    /** The form of an id or tag that is sent: bytes outside {@code [A-Za-z0-9._:/-]} become {@code _}, at most {@code maxBytes}. */
    static String sanitizeIdent(String text, int maxBytes) {
        return sent(text, maxBytes, false);
    }

    static String ident(String text, int maxBytes) {
        return '"' + sanitizeIdent(text, maxBytes) + '"';
    }

    /** Like an id, but {@code #} is allowed (claim-order keys are {@code t#<n>}). */
    static String threadKey(String text, int maxBytes) {
        return '"' + sent(text, maxBytes, true) + '"';
    }

    /** The quoted key followed by spaces up to 64 key bytes, so a line's length does not depend on the key. */
    static String threadKeyPadded(String text) {
        String quoted = threadKey(text, THREAD_KEY_MAX);
        int used = quoted.length() - 2;
        return used >= THREAD_KEY_MAX ? quoted : quoted + " ".repeat(THREAD_KEY_MAX - used);
    }

    static long fnv1a64(byte[] data) {
        long value = FNV_OFFSET;
        for (byte b : data) {
            value = (value ^ (b & 0xffL)) * FNV_PRIME;
        }
        return value;
    }

    /** FNV-1a 64 over the form of an id that is sent. */
    static long identHash(String text, int maxBytes) {
        return fnv1a64(sanitizeIdent(text, maxBytes).getBytes(StandardCharsets.US_ASCII));
    }
}
