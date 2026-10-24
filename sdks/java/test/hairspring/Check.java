package hairspring;

import java.util.Arrays;
import java.util.Objects;

/** Assertions for the self-test: a failed one throws {@link AssertionError}, which the suite reports. */
final class Check {
    private Check() {}

    /** A test body. */
    interface Case {
        void run() throws Exception;
    }

    static void equal(Object want, Object got) {
        equal("values", want, got);
    }

    static void equal(String what, Object want, Object got) {
        if (!Objects.deepEquals(want, got)) {
            throw new AssertionError(what + ": want " + show(want) + ", got " + show(got));
        }
    }

    static void yes(boolean condition, String what) {
        if (!condition) {
            throw new AssertionError("not true: " + what);
        }
    }

    static void no(boolean condition, String what) {
        yes(!condition, "not (" + what + ")");
    }

    /** Runs {@code body}, which must throw {@code type}; returns what it threw. */
    static <T extends Throwable> T raises(Class<T> type, Case body) {
        try {
            body.run();
        } catch (Throwable thrown) {
            if (type.isInstance(thrown)) {
                return type.cast(thrown);
            }
            throw new AssertionError("want " + type.getSimpleName() + ", got " + thrown, thrown);
        }
        throw new AssertionError("want " + type.getSimpleName() + ", nothing was thrown");
    }

    static String show(Object value) {
        if (value instanceof long[] longs) {
            return Arrays.toString(longs);
        }
        if (value instanceof byte[] bytes) {
            return "bytes(" + new String(bytes, java.nio.charset.StandardCharsets.ISO_8859_1) + ")";
        }
        return String.valueOf(value);
    }
}
