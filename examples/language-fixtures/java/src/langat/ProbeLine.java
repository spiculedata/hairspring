package langat;

import java.util.Locale;

/** The one stdout line. */
final class ProbeLine {
    private ProbeLine() {}

    /**
     * {@code PROBE v1 lang=java lock_order=<16 hex> arrival=<16 hex> rand=<16 hex> vtime_ns=<20 digits> extra=<16 hex>}
     * and a newline. A negative time is written as zero; hex is unsigned.
     */
    static String format(Fields fields) {
        return String.format(
                Locale.ROOT,
                "PROBE v1 lang=java lock_order=%016x arrival=%016x rand=%016x vtime_ns=%020d extra=%016x\n",
                fields.lockOrder(),
                fields.arrival(),
                fields.rand(),
                Math.max(fields.vtimeNs(), 0L),
                fields.extra());
    }
}
