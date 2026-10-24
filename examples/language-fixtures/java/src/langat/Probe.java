package langat;

import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;

/**
 * The Java feasibility probe: no SDK, no property, no arguments. Four platform threads share a monitor, a counter with no
 * guard and a queue; a map gives a fourth structure. It prints exactly one line on stdout, with one write call, and
 * exits 0:
 *
 * <pre>PROBE v1 lang=java lock_order=&lt;16 hex&gt; arrival=&lt;16 hex&gt; rand=&lt;16 hex&gt; vtime_ns=&lt;20 digits&gt; extra=&lt;16 hex&gt;</pre>
 *
 * {@code lock_order} and {@code arrival} are FNV-1a 64 hashes of the worker numbers written as bare digits, {@code extra}
 * hashes the map's keys in iteration order, {@code rand} is the first value of a new {@link java.util.Random}, and
 * {@code vtime_ns} is the monotonic clock difference between the start and the end of the run. Exit 3 is an internal
 * invariant failure (a thread did not start or died, the line could not be written); exit 64 is a usage error (any
 * argument).
 */
public final class Probe {
    static final int EXIT_INVARIANT = 3;
    static final int EXIT_USAGE = 64;
    private static final String USAGE = "usage: java -cp lang-at.jar langat.Probe (no arguments)";

    private Probe() {}

    public static void main(String[] args) {
        System.exit(run(args, new FileOutputStream(FileDescriptor.out), System.err));
    }

    /** Runs the probe against the given streams and returns the exit status. */
    static int run(String[] args, OutputStream out, PrintStream err) {
        if (args.length > 0) {
            err.println(USAGE);
            return EXIT_USAGE;
        }
        try {
            byte[] line = ProbeLine.format(fieldsOf(Workload.run())).getBytes(StandardCharsets.US_ASCII);
            // One call, then a flush: the whole line is one write(2) on the real stdout.
            out.write(line, 0, line.length);
            out.flush();
            return 0;
        } catch (InvariantException failed) {
            err.println("probe: internal invariant failed: " + failed.getMessage());
        } catch (IOException failed) {
            err.println("probe: the stdout line could not be written: " + failed.getMessage());
        } catch (RuntimeException failed) {
            err.println("probe: internal failure: " + failed);
        }
        return EXIT_INVARIANT;
    }

    static Fields fieldsOf(Outcome outcome) {
        return new Fields(
                Fnv.hashDigits(outcome.lockOrder()),
                Fnv.hashDigits(outcome.arrival()),
                outcome.rand(),
                outcome.elapsedNs(),
                Fnv.hashSequence(outcome.mapOrder()));
    }
}
