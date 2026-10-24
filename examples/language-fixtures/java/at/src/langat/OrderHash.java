package langat;

import java.nio.charset.StandardCharsets;

/** The FNV-1a 64 hash of the arrival list written as ASCII digits: the {@code order} field of the result line. */
final class OrderHash {
    private static final long OFFSET = 0xcbf29ce484222325L;
    private static final long PRIME = 0x00000100000001b3L;

    private OrderHash() {}

    static long fnv1a64(byte[] bytes) {
        long hash = OFFSET;
        for (byte one : bytes) {
            hash ^= one & 0xffL;
            hash *= PRIME;
        }
        return hash;
    }

    static long of(int[] arrival) {
        StringBuilder digits = new StringBuilder();
        for (int worker : arrival) {
            digits.append(worker);
        }
        return fnv1a64(digits.toString().getBytes(StandardCharsets.US_ASCII));
    }
}
