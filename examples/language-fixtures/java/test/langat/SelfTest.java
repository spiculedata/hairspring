package langat;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import java.util.regex.Pattern;

/**
 * Self-test of the probe's classes, with no test framework. It exits 1 when any check fails.
 *
 * <p>With {@code --order-hashes} it instead prints the 24 hashes a thread order can have (every permutation of the
 * worker numbers 0 to 3), one per line, for the shell test that checks the probe's output.
 */
public final class SelfTest {
    private static final Pattern LINE = Pattern.compile(
            "PROBE v1 lang=java lock_order=[0-9a-f]{16} arrival=[0-9a-f]{16} rand=[0-9a-f]{16}"
                    + " vtime_ns=[0-9]{20} extra=[0-9a-f]{16}\n");
    private static final int THREADS = 4;
    private static final int ROUNDS = 200;
    private static final int MAP_ENTRIES = 32;

    private static int failures = 0;

    private interface Check {
        void run() throws Exception;
    }

    private SelfTest() {}

    public static void main(String[] args) {
        if (args.length == 1 && args[0].equals("--order-hashes")) {
            printOrderHashes();
            return;
        }
        fnvTests();
        lineTests();
        workloadTests();
        probeTests();
        System.out.println(failures == 0 ? "selftest ok" : "selftest FAILED: " + failures);
        System.exit(failures == 0 ? 0 : 1);
    }

    private static void printOrderHashes() {
        for (List<Integer> order : permutations(List.of(0, 1, 2, 3))) {
            System.out.println(String.format("%016x", Fnv.hashDigits(order)));
        }
    }

    private static List<List<Integer>> permutations(List<Integer> items) {
        if (items.size() <= 1) {
            return List.of(items);
        }
        List<List<Integer>> all = new ArrayList<>();
        for (int i = 0; i < items.size(); i++) {
            List<Integer> rest = new ArrayList<>(items);
            Integer head = rest.remove(i);
            for (List<Integer> tail : permutations(rest)) {
                List<Integer> one = new ArrayList<>();
                one.add(head);
                one.addAll(tail);
                all.add(one);
            }
        }
        return all;
    }

    private static void test(String name, Check body) {
        try {
            body.run();
            System.out.println("ok   " + name);
        } catch (Throwable problem) {
            failures++;
            System.out.println("FAIL " + name + ": " + problem);
        }
    }

    private static void require(boolean condition, String what) {
        if (!condition) {
            throw new AssertionError(what);
        }
    }

    private static void same(Object wanted, Object got) {
        require(wanted.equals(got), "wanted " + wanted + ", got " + got);
    }

    private static byte[] ascii(String text) {
        return text.getBytes(StandardCharsets.US_ASCII);
    }

    private static void fnvTests() {
        test("fnv1a64 matches the published vectors", () -> {
            same(0xcbf29ce484222325L, Fnv.fnv1a64(ascii("")));
            same(0xaf63dc4c8601ec8cL, Fnv.fnv1a64(ascii("a")));
            same(0x85944171f73967e8L, Fnv.fnv1a64(ascii("foobar")));
        });
        test("a thread order hashes as bare ASCII digits", () -> {
            same(Fnv.fnv1a64(ascii("2013")), Fnv.hashDigits(List.of(2, 0, 1, 3)));
            require(Fnv.hashDigits(List.of(0, 1, 2, 3)) != Fnv.hashDigits(List.of(3, 2, 1, 0)), "order must matter");
        });
        test("a sequence hashes as its items each followed by a comma", () -> {
            same(Fnv.fnv1a64(ascii("k2,k0,k1,")), Fnv.hashSequence(List.of("k2", "k0", "k1")));
            require(Fnv.hashSequence(List.of("a", "bc")) != Fnv.hashSequence(List.of("ab", "c")), "boundaries must count");
            same(Fnv.fnv1a64(ascii("")), Fnv.hashSequence(List.of()));
        });
    }

    private static void lineTests() {
        test("the line has the contract form", () -> {
            Fields fields = new Fields(0xabL, 0xcdL, 0x1234L, 5_000_000L, 0xffffffffffffffffL);
            same("PROBE v1 lang=java lock_order=00000000000000ab arrival=00000000000000cd rand=0000000000001234"
                    + " vtime_ns=00000000000005000000 extra=ffffffffffffffff\n", ProbeLine.format(fields));
        });
        test("the largest signed time still has twenty digits", () -> {
            String line = ProbeLine.format(new Fields(0, 0, 0, Long.MAX_VALUE, 0));
            require(line.contains(" vtime_ns=09223372036854775807 "), line);
        });
        test("a negative time is written as zero, never with a sign", () -> {
            String line = ProbeLine.format(new Fields(0, 0, 0, -5L, 0));
            require(line.contains(" vtime_ns=00000000000000000000 "), line);
        });
        test("a negative random value is written as unsigned hex", () -> {
            String line = ProbeLine.format(new Fields(0, 0, -1L, 1, 0));
            require(line.contains(" rand=ffffffffffffffff "), line);
        });
    }

    private static void workloadTests() {
        test("a run takes the lock and finishes once per thread", () -> {
            Outcome outcome = Workload.run();
            Set<Integer> all = Set.of(0, 1, 2, 3);
            same(all, new HashSet<>(outcome.lockOrder()));
            same(all, new HashSet<>(outcome.arrival()));
            same(THREADS, outcome.lockOrder().size());
            same(THREADS, outcome.arrival().size());
        });
        test("the racy counter ends between one and every round, the synchronized one at every round", () -> {
            Outcome outcome = Workload.run();
            require(outcome.racyFinal() >= 1 && outcome.racyFinal() <= (long) THREADS * ROUNDS, "racy " + outcome);
            same((long) THREADS * ROUNDS, outcome.lockedFinal());
        });
        test("a run reports a positive elapsed time", () -> require(Workload.run().elapsedNs() > 0, "elapsed"));
        test("the map order holds every key once and is the same every time", () -> {
            List<String> first = Workload.mapOrder();
            same(MAP_ENTRIES, new HashSet<>(first).size());
            same(MAP_ENTRIES, first.size());
            same(first, Workload.mapOrder());
        });
        test("the spin depends on its count and is repeatable", () -> {
            same(Workload.spin(100), Workload.spin(100));
            require(Workload.spin(100) != Workload.spin(101), "count must matter");
            require(Workload.spin(0) != Workload.spin(1), "zero rounds differ from one");
        });
    }

    /** Records every call to write, so the test can count them. */
    private static final class Recording extends OutputStream {
        private final List<byte[]> writes = new ArrayList<>();
        private int flushes = 0;

        @Override
        public void write(int one) {
            writes.add(new byte[] {(byte) one});
        }

        @Override
        public void write(byte[] bytes, int offset, int length) {
            byte[] copy = new byte[length];
            System.arraycopy(bytes, offset, copy, 0, length);
            writes.add(copy);
        }

        @Override
        public void flush() {
            flushes++;
        }
    }

    private static final class Broken extends OutputStream {
        @Override
        public void write(int one) throws IOException {
            throw new IOException("no space left on device");
        }
    }

    private static int probe(String[] args, OutputStream out, ByteArrayOutputStream err) {
        return Probe.run(args, out, new PrintStream(err, true, StandardCharsets.US_ASCII));
    }

    private static void probeTests() {
        test("a run writes the whole line in one write call, then flushes, and exits 0", () -> {
            Recording out = new Recording();
            ByteArrayOutputStream err = new ByteArrayOutputStream();
            same(0, probe(new String[0], out, err));
            same(1, out.writes.size());
            require(out.flushes >= 1, "no flush");
            String line = new String(out.writes.get(0), StandardCharsets.US_ASCII);
            require(LINE.matcher(line).matches(), "not the contract line: " + line);
            same(0, err.size());
        });
        test("any argument is a usage error: exit 64, a usage line on stderr, nothing on stdout", () -> {
            for (String arg : new String[] {"x", "", "--switch=1.0", "buggy"}) {
                Recording out = new Recording();
                ByteArrayOutputStream err = new ByteArrayOutputStream();
                same(64, probe(new String[] {arg}, out, err));
                same(0, out.writes.size());
                require(err.size() > 0, "no usage line for '" + arg + "'");
            }
            same(64, probe(new String[] {"a", "b"}, new Recording(), new ByteArrayOutputStream()));
        });
        test("a stdout that cannot be written is an internal failure: exit 3 and a message", () -> {
            ByteArrayOutputStream err = new ByteArrayOutputStream();
            same(3, probe(new String[0], new Broken(), err));
            require(err.size() > 0, "no message");
        });
        test("an Error or a runtime exception from the workload is an internal failure: exit 3 and a message", () -> {
            Probe.Runner[] failing = {
                () -> {
                    throw new OutOfMemoryError("unable to create native thread");
                },
                () -> {
                    throw new StackOverflowError();
                },
                () -> {
                    throw new IllegalStateException("boom");
                },
                () -> {
                    throw new InvariantException("a worker died");
                }
            };
            for (Probe.Runner runner : failing) {
                Recording out = new Recording();
                ByteArrayOutputStream err = new ByteArrayOutputStream();
                same(3, Probe.run(new String[0], out, new PrintStream(err, true, StandardCharsets.US_ASCII), runner));
                same(0, out.writes.size());
                require(err.size() > 0, "no message");
            }
        });
        test("two runs agree on the fields that do not depend on the schedule", () -> {
            String first = lineOf();
            String second = lineOf();
            same(field(first, "extra"), field(second, "extra"));
            same(field(first, "lang"), "java");
        });
    }

    private static String lineOf() {
        Recording out = new Recording();
        probe(new String[0], out, new ByteArrayOutputStream());
        return new String(out.writes.get(0), StandardCharsets.US_ASCII);
    }

    private static String field(String line, String name) {
        for (String token : line.trim().split(" ")) {
            if (token.startsWith(name + "=")) {
                return token.substring(name.length() + 1);
            }
        }
        throw new AssertionError("no field " + name + " in " + line);
    }
}
