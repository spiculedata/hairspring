package hssample;

import hairspring.Hairspring;
import java.util.Arrays;

/**
 * Sample instrumented programs, one per scenario: {@code java -cp <sdk>:<samples> hssample.Sample <scenario> [args]}.
 *
 * <p>The process tests (test/test-process.sh) and the Rust conformance test
 * (crates/hairspring-props/tests/java_sdk.rs) run them and inspect the events they write. {@code basic} has the same
 * ids, messages, values and order as the C SDK's {@code basic} scenario (sdks/c/tests/fixtures/hs_sample.c), so the two
 * SDKs' reports can be compared line by line; the three sites the C program never reaches are declared here, because
 * Java cannot see a call site that has not run.
 */
public final class Sample {
    private static final int EXIT_USAGE = 64;

    private Sample() {}

    public static void main(String[] args) throws Exception {
        if (args.length < 1 || !Scenarios.NAMES.contains(args[0])) {
            System.err.println("usage: Sample <" + String.join("|", Scenarios.NAMES) + "> [args]");
            Hairspring.exit(EXIT_USAGE);
        }
        int status = Scenarios.run(args[0], Arrays.copyOfRange(args, 1, args.length));
        if (status != 0) {
            Hairspring.exit(status);
        }
    }
}
