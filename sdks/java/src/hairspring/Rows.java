package hairspring;

/** The immutable values the protocol lines are made from. Nothing here reads the process. */
final class Rows {
    private Rows() {}

    /** What the lines say about the process. */
    record Proc(String tag, boolean tagStable, boolean signalHandlers, long catalogHash, int nIds, long dropped) {}

    record CatalogRow(String id, Kind kind, String msg, String file, long line) {}

    /** A witness ready to print: the thread's key, that thread's own ordinal and four values. */
    record Witness(String key, long ord, long[] vals) {}

    record AssertRow(String id, Kind kind, long trues, long falses, Witness firstTrue, Witness firstFalse) {}

    record ThreadRow(String key, long evals, long tapeConsumed, boolean stable) {}
}
