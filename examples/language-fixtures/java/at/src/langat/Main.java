package langat;

import hairspring.Hairspring;
import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.util.Optional;
import java.util.Random;
import java.util.function.LongSupplier;

/**
 * The Java lost-update fixture: one program with three modes chosen by its first argument ({@code buggy}, {@code fixed},
 * {@code wide}). Four threads each do 1,000 times: load a shared counter, run a busy loop, store the value read plus one.
 * It reports through the Hairspring Java SDK: {@code java.counter_exact} ({@code always}, the final counter is 4,000, with
 * the witness values {@code [final, 4000, 4, 1000]}) and {@code java.all_started} ({@code sometimes}), each evaluated once
 * in the main thread after the workers are joined. Stdout gets one line, in one write:
 *
 * <pre>LANGAT v1 lang=java mode=&lt;mode&gt; final=&lt;20 digits&gt; expect=&lt;20 digits&gt; order=&lt;16 hex&gt; rand=&lt;16 hex&gt;</pre>
 *
 * <p>Exit status: 0 whatever the property says; 3 if the program itself failed (a thread did not start, the barrier broke,
 * the arrival list is not four entries long, the line could not be written); 64 for a usage error. The verdict is the
 * SDK's report, not the exit code.
 */
public final class Main {
    private static final String USAGE = "usage: java -cp lang-at.jar langat.Main buggy|fixed|wide";

    private Main() {}

    /** The workers, as a seam so a test can make them fail. */
    interface Workload {
        RaceResult run(Mode mode) throws RaceFailure;
    }

    public static void main(String[] args) {
        int status = run(
                args,
                new FileOutputStream(FileDescriptor.out),
                System.err,
                mode -> Race.run(mode, Contract.ITERATIONS),
                () -> new Random().nextLong());
        if (status != Contract.EXIT_OK) {
            Hairspring.exit(status); // through the SDK, so the end line carries the same status as the process
        }
    }

    /** Runs the program against the given streams and returns the exit status. */
    static int run(String[] args, OutputStream out, PrintStream err, Workload workload, LongSupplier rand) {
        Optional<Mode> parsed = Mode.parse(args);
        if (parsed.isEmpty()) {
            err.println(USAGE);
            return Contract.EXIT_USAGE;
        }
        Mode mode = parsed.get();
        RaceResult result;
        try {
            result = workload.run(mode);
        } catch (RaceFailure failed) {
            err.println("invariant failure: " + failed.getMessage());
            return Contract.EXIT_INVARIANT;
        }
        if (result.arrival().length != Contract.THREADS) {
            err.println("invariant failure: the arrival list does not hold " + Contract.THREADS + " entries");
            return Contract.EXIT_INVARIANT;
        }
        long finalValue = result.finalValue();
        Hairspring.always(
                finalValue == Contract.EXPECTED,
                Contract.LANG + ".counter_exact",
                "counter equals T*K",
                finalValue,
                Contract.EXPECTED,
                Contract.THREADS,
                Contract.ITERATIONS);
        Hairspring.sometimes(
                result.started() == Contract.THREADS, Contract.LANG + ".all_started", "all workers started", result.started());
        byte[] line = LangAtLine.format(mode, finalValue, OrderHash.of(result.arrival()), rand.getAsLong())
                .getBytes(StandardCharsets.US_ASCII);
        try {
            // One call, then a flush: the whole line is one write(2) on the real stdout.
            out.write(line, 0, line.length);
            out.flush();
        } catch (IOException failed) {
            err.println("invariant failure: the stdout line could not be written: " + failed.getMessage());
            return Contract.EXIT_INVARIANT;
        }
        return Contract.EXIT_OK;
    }
}
