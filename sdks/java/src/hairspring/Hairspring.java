package hairspring;

import java.util.Map;

/**
 * Hairspring guest SDK for Java: in-source property assertions for programs under test, in guest event protocol v1
 * ({@code docs/guest-protocol.md}). JDK classes only; Java 21; Linux and macOS.
 *
 * <pre>{@code
 * import static hairspring.Hairspring.*;
 *
 * always(balance >= 0, "bank.balance_non_negative", "no overdraft", balance);
 * sometimes(waited, "bank.writer.waited", "a writer waited for the lock");
 * reachable("bank.recovery.ran", "recovery ran");
 * unreachable("bank.bad_state", "never expected");
 * work(1);                      // operations done, reported as work.ops
 * thread("writer", 0);          // a stable thread key "writer-0"
 * }</pre>
 *
 * <p>A call site only counts in memory; the program reports once, from a JVM shutdown hook or by {@link #report()}. The
 * SDK starts when this class is first used (any call, or {@link #start()}); a program that never touches it reports
 * nothing. Java cannot see a call site before it runs, so a site in code the run never reaches is invisible, not unmet:
 * {@link #declare} every site whose absence matters.
 *
 * <p>A site is catalogued the first time it is evaluated. The same id and kind from several places is one site.
 * {@code id} is 1 to 128 bytes of UTF-8 (bytes outside {@code [A-Za-z0-9._:/-]} are sent as {@code _}); {@code msg} is at
 * most 512 bytes; {@code vals} are up to four values, captured at the first true and the first false evaluation (missing
 * values are 0). An invalid argument throws {@link IllegalArgumentException} (or {@link NullPointerException}) and counts
 * nothing.
 */
public final class Hairspring {
    private static final long[] NO_VALS = new long[0];
    private static final Recorder RECORDER = Recorder.start();

    private Hairspring() {}

    /** Starts the SDK (it also starts at the first call) so the process reports even if no site ever runs. */
    public static void start() {
        // Loading this class ran the static initializer, which started the recorder and registered the shutdown hook.
    }

    /** {@code cond} must hold at every evaluation. Trial-level failure: any false. Run-level UNMET: never evaluated. */
    public static void always(boolean cond, String id, String msg, long... vals) {
        RECORDER.record(Kind.ALWAYS, cond, id, msg, vals);
    }

    public static void always(boolean cond, String id) {
        RECORDER.record(Kind.ALWAYS, cond, id, "", NO_VALS);
    }

    /** Like {@link #always}, but never reaching it is fine (no UNMET). */
    public static void alwaysOrUnreachable(boolean cond, String id, String msg, long... vals) {
        RECORDER.record(Kind.ALWAYS_OR_UNREACHABLE, cond, id, msg, vals);
    }

    public static void alwaysOrUnreachable(boolean cond, String id) {
        RECORDER.record(Kind.ALWAYS_OR_UNREACHABLE, cond, id, "", NO_VALS);
    }

    /** {@code cond} must hold at least once per run. No trial-level failure; UNMET if never true. */
    public static void sometimes(boolean cond, String id, String msg, long... vals) {
        RECORDER.record(Kind.SOMETIMES, cond, id, msg, vals);
    }

    public static void sometimes(boolean cond, String id) {
        RECORDER.record(Kind.SOMETIMES, cond, id, "", NO_VALS);
    }

    /** This point must be reached at least once per run. No trial-level failure; UNMET if never hit. */
    public static void reachable(String id, String msg, long... vals) {
        RECORDER.record(Kind.REACHABLE, true, id, msg, vals);
    }

    public static void reachable(String id) {
        RECORDER.record(Kind.REACHABLE, true, id, "", NO_VALS);
    }

    /** This point must never be reached. Trial-level failure: hit at least once. */
    public static void unreachable(String id, String msg, long... vals) {
        RECORDER.record(Kind.UNREACHABLE, true, id, msg, vals);
    }

    public static void unreachable(String id) {
        RECORDER.record(Kind.UNREACHABLE, true, id, "", NO_VALS);
    }

    /**
     * Put a site in the catalog without evaluating it. Java cannot see a call site that has not run, so a
     * {@code reachable} or {@code sometimes} that is never reached would be invisible, and the run could not report it as
     * unmet. Declare such sites once, early; evaluating the same id and kind later is the same site.
     */
    public static void declare(Kind kind, String id, String msg) {
        RECORDER.declare(kind, id, msg);
    }

    /** Add {@code n} to the calling thread's operation count; the report sends the total as {@code work.ops}. */
    public static void work(long n) {
        RECORDER.work(n);
    }

    /**
     * Give the calling thread the key {@code label-index}. Returns false (and the thread keeps its claim-order key
     * {@code t#<n>}, flagged {@code key_stable:false}) when another thread has that key or the thread shares the overflow
     * slot. Throws {@link IllegalArgumentException} for an empty label, a bad index or a key over 64 bytes. Call it before
     * the thread does anything else.
     */
    public static boolean thread(String label, long index) {
        return RECORDER.thread(label, index);
    }

    /** Write the report now (reason {@code explicit}). The first report wins; evaluations after it are not reported. */
    public static void report() {
        RECORDER.report(EndReason.EXPLICIT, null);
    }

    /**
     * Record {@code status} and exit through {@link System#exit}, so the shutdown hook's report carries it. Do not call it
     * from a shutdown hook.
     */
    public static void exit(int status) {
        RECORDER.setStatus(status);
        System.exit(status);
    }

    /**
     * Puts the tag and the owning pid in the environment of a process you are about to start, so that the child (if it
     * also uses this SDK) reports under {@code <tag>.s<pid>} instead of repeating this process's tag. Java cannot change its
     * own environment, so this has to be said for each {@link ProcessBuilder}. Returns the builder.
     */
    public static ProcessBuilder prepare(ProcessBuilder builder) {
        Map<String, String> environment = builder.environment();
        environment.putAll(RECORDER.childEnvironment());
        return builder;
    }
}
