package hairspring;

import java.util.Locale;

/**
 * The five assertion kinds. The numbers are the C SDK's ({@code enum hs_kind}): they are part of the catalog hash.
 *
 * <pre>
 * kind                  trial-level failure     run-level UNMET
 * ALWAYS                any evaluation false    never evaluated
 * ALWAYS_OR_UNREACHABLE any evaluation false    none
 * SOMETIMES             none                    never true
 * REACHABLE             none                    never hit
 * UNREACHABLE           hit at least once       none
 * </pre>
 */
public enum Kind {
    ALWAYS(1, true, false),
    ALWAYS_OR_UNREACHABLE(2, false, false),
    SOMETIMES(3, true, false),
    REACHABLE(4, true, true),
    UNREACHABLE(5, false, true);

    private final int number;
    private final boolean mustHit;
    private final boolean point;

    Kind(int number, boolean mustHit, boolean point) {
        this.number = number;
        this.mustHit = mustHit;
        this.point = point;
    }

    /** The C SDK's number for this kind. */
    int number() {
        return number;
    }

    /** The kind's name on the wire ({@code type}). */
    String wireName() {
        return name().toLowerCase(Locale.ROOT);
    }

    /** Whether the run-level result is UNMET when the site never succeeds ({@code must_hit}). */
    boolean mustHit() {
        return mustHit;
    }

    /** A point has no condition: reaching it is the event ({@code reachable}, {@code unreachable}). */
    boolean isPoint() {
        return point;
    }
}
