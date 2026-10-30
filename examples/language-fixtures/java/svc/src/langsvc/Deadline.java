package langsvc;

/**
 * The only clock reads in the service: a request's deadline and the check of it. Under Hermit these read virtual time.
 * The source check allows the monotonic clock in this file and nowhere else.
 */
final class Deadline {
    private Deadline() {}

    /** A deadline {@code ttlMs} from now, on the monotonic clock. */
    static long after(long ttlMs) {
        return System.nanoTime() + ttlMs * 1_000_000L;
    }

    /** True once the monotonic clock has passed {@code deadline}. */
    static boolean passed(long deadline) {
        return System.nanoTime() - deadline > 0;
    }
}
