package hairspring;

/** One thread's counters. Every field is changed only with the recorder's lock held. */
final class Slot {
    final int index;
    String key;
    boolean stable;
    long evals;
    long work;

    Slot(int index, String key) {
        this.index = index;
        this.key = key;
    }
}
