package hairspring;

import java.util.Objects;

/** Validation of what a call site passes. Called before any state changes, so a bad argument counts nothing. */
final class Args {
    static final int MAX_VALS = 4;
    /** A UTF-16 unit is at most three bytes of UTF-8 (a surrogate pair is four bytes for two units). */
    private static final int MAX_UTF8_BYTES_PER_UNIT = 3;

    private Args() {}

    static void checkId(String raw) {
        Objects.requireNonNull(raw, "an assertion id must not be null");
        int size = Fmt.utf8(raw).length;
        if (size < 1 || size > Fmt.ID_MAX) {
            throw new IllegalArgumentException("an assertion id is 1 to " + Fmt.ID_MAX + " bytes, not " + size);
        }
        if (raw.indexOf('\0') >= 0) {
            // The wire form stops at a NUL, so the id would be sent shortened or empty.
            throw new IllegalArgumentException("an assertion id must not contain a NUL character");
        }
    }

    static void checkMsg(String raw) {
        Objects.requireNonNull(raw, "an assertion message must not be null");
        if ((long) raw.length() * MAX_UTF8_BYTES_PER_UNIT <= Fmt.MSG_MAX) {
            return; // short enough that it cannot be over the limit: no encoding, so a known site's call allocates nothing
        }
        int size = Fmt.utf8(raw).length;
        if (size > Fmt.MSG_MAX) {
            throw new IllegalArgumentException("an assertion message is at most " + Fmt.MSG_MAX + " bytes, not " + size);
        }
    }

    /** Fails on a fifth value; cheap enough for every call. */
    static void checkVals(long[] vals) {
        Objects.requireNonNull(vals, "the values must not be null");
        if (vals.length > MAX_VALS) {
            throw new IllegalArgumentException("an assertion takes at most " + MAX_VALS + " values, not " + vals.length);
        }
    }

    /** Up to four values, zero-filled. */
    static long[] packVals(long[] vals) {
        checkVals(vals);
        long[] packed = new long[MAX_VALS];
        System.arraycopy(vals, 0, packed, 0, vals.length);
        return packed;
    }
}
