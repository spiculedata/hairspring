package hairspring;

import java.util.List;

/**
 * An immutable snapshot of the recorder's state, taken with its lock held, so every count, witness and thread total in
 * it belongs to one moment.
 */
record Snapshot(
        List<Rows.CatalogRow> catalog,
        List<Rows.AssertRow> asserts,
        List<ThreadView> threads,
        long catalogHash,
        int nIds,
        long dropped) {

    /** The operations all threads counted. */
    long work() {
        long total = 0;
        for (ThreadView t : threads) {
            total += t.work();
        }
        return total;
    }

    /** The evaluations all threads counted. */
    long evals() {
        long total = 0;
        for (ThreadView t : threads) {
            total += t.evals();
        }
        return total;
    }
}
