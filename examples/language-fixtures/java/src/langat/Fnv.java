package langat;

import java.nio.charset.StandardCharsets;
import java.util.List;

/** FNV-1a 64 and the two ways the probe hashes an order. */
final class Fnv {
    private static final long OFFSET = 0xcbf29ce484222325L;
    private static final long PRIME = 0x00000100000001b3L;
    private static final int DECIMAL = 10;

    private Fnv() {}

    /** FNV-1a 64 of {@code bytes}. */
    static long fnv1a64(byte[] bytes) {
        long hash = OFFSET;
        for (byte one : bytes) {
            hash ^= one & 0xffL;
            hash *= PRIME;
        }
        return hash;
    }

    /** The hash of a thread order (worker numbers 0 to 3) written as bare ASCII digits, like the fixtures' order hash. */
    static long hashDigits(List<Integer> workers) {
        StringBuilder digits = new StringBuilder();
        for (int worker : workers) {
            digits.append((char) ('0' + worker % DECIMAL));
        }
        return fnv1a64(digits.toString().getBytes(StandardCharsets.US_ASCII));
    }

    /** The hash of a sequence of keys: each key followed by a comma, so item boundaries count. */
    static long hashSequence(List<String> items) {
        StringBuilder text = new StringBuilder();
        for (String item : items) {
            text.append(item).append(',');
        }
        return fnv1a64(text.toString().getBytes(StandardCharsets.US_ASCII));
    }
}
