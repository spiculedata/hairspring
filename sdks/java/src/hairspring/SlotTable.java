package hairspring;

import java.util.ArrayList;
import java.util.List;
import java.util.Objects;

/**
 * The table of per-thread slots. A thread claims one on its first SDK call; slots are never recycled and keys cannot be
 * reused, so the table suits fixed pools. Past the 255th thread the rest share the last slot.
 *
 * <p>The table does no locking of its own: the recorder calls it with its lock held.
 */
final class SlotTable {
    static final int MAX_THREADS = 256;
    static final int OVERFLOW_SLOT = MAX_THREADS - 1;
    static final String OVERFLOW_KEY = "t#overflow";
    static final long MAX_INDEX = 4294967295L;

    /** The slot a claim got; {@code shared} says the thread shares the overflow slot. */
    record Claim(Slot slot, boolean shared) {}

    private final List<Slot> slots = new ArrayList<>();
    private int claims;

    int used() {
        return slots.size();
    }

    Claim claim() {
        int n = claims++;
        if (n < OVERFLOW_SLOT) {
            Slot slot = new Slot(n, "t#" + (n + 1));
            slots.add(slot);
            return new Claim(slot, false);
        }
        if (n == OVERFLOW_SLOT) {
            slots.add(new Slot(OVERFLOW_SLOT, OVERFLOW_KEY));
        }
        return new Claim(slots.get(OVERFLOW_SLOT), true);
    }

    String keyAt(int index) {
        return slots.isEmpty() ? "" : slots.get(Math.min(index, slots.size() - 1)).key;
    }

    /**
     * Give {@code slot} the key {@code label-index}. Bytes of the label outside {@code [A-Za-z0-9._:/-]} become
     * {@code _}. An empty label or a key over 64 bytes is a programming error ({@link IllegalArgumentException}); a key
     * another thread has, or a thread that shares the overflow slot, is refused with {@code false} and the thread keeps
     * its claim-order key.
     */
    boolean setKey(Slot slot, String label, long index) {
        Objects.requireNonNull(label, "a thread label must not be null");
        if (label.isEmpty()) {
            throw new IllegalArgumentException("a thread label must not be empty");
        }
        if (index < 0 || index > MAX_INDEX) {
            throw new IllegalArgumentException("a thread index must be between 0 and " + MAX_INDEX);
        }
        String key = Fmt.sanitizeIdent(label, Fmt.utf8(label).length) + "-" + index;
        if (key.length() > Fmt.THREAD_KEY_MAX) {
            throw new IllegalArgumentException("the key " + key + " is longer than " + Fmt.THREAD_KEY_MAX + " bytes");
        }
        if (slot.index == OVERFLOW_SLOT) {
            return false;
        }
        for (Slot other : slots) {
            if (other != slot && other.key.equals(key)) {
                return false;
            }
        }
        slot.key = key;
        slot.stable = true;
        return true;
    }

    List<ThreadView> views() {
        List<ThreadView> views = new ArrayList<>(slots.size());
        for (Slot s : slots) {
            views.add(new ThreadView(s.key, s.evals, s.work, 0, s.stable));
        }
        return List.copyOf(views);
    }
}
