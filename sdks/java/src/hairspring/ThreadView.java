package hairspring;

/** An immutable copy of a {@link Slot}, for the report. */
record ThreadView(String key, long evals, long work, long tapeConsumed, boolean stable) {}
