package langat;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Optional;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * The fixture's own tests: its command line, its line, its barrier, its invariants and its workers. No framework, one
 * process; run with {@code HAIRSPRING_EVENTS=off} so the SDK writes no report for the evaluations these tests make. Exits
 * 0 when every case passed, 1 otherwise.
 */
public final class AtSelfTest {
    private interface Case {
        void run() throws Exception;
    }

    private static int failed;
    private static int total;

    private AtSelfTest() {}

    private static void check(String name, Case body) {
        total++;
        try {
            body.run();
            System.out.println("ok   " + name);
        } catch (Throwable problem) {
            failed++;
            System.out.println("FAIL " + name + ": " + problem);
            if (!(problem instanceof AssertionError)) {
                problem.printStackTrace(System.out);
            }
        }
    }

    private static void equal(Object want, Object got) {
        if (!java.util.Objects.deepEquals(want, got)) {
            throw new AssertionError("want " + show(want) + ", got " + show(got));
        }
    }

    private static void yes(boolean condition, String what) {
        if (!condition) {
            throw new AssertionError("not true: " + what);
        }
    }

    private static String show(Object value) {
        return value instanceof int[] ints ? Arrays.toString(ints) : String.valueOf(value);
    }

    private static Optional<Mode> parse(String... args) {
        return Invocation.parse(args).invocation().map(Invocation::mode);
    }

    /** An output stream that records how many write calls it saw and what they held. */
    private static final class Recording extends OutputStream {
        final List<byte[]> writes = new ArrayList<>();

        @Override
        public void write(int b) {
            writes.add(new byte[] {(byte) b});
        }

        @Override
        public void write(byte[] bytes, int offset, int length) {
            writes.add(Arrays.copyOfRange(bytes, offset, offset + length));
        }

        String text() {
            ByteArrayOutputStream all = new ByteArrayOutputStream();
            writes.forEach(w -> all.write(w, 0, w.length));
            return all.toString(StandardCharsets.US_ASCII);
        }
    }

    private static final class Unwritable extends OutputStream {
        @Override
        public void write(int b) throws IOException {
            throw new IOException("No space left on device");
        }
    }

    private static Invocation.Parsed invoke(String... args) {
        return Invocation.parse(args);
    }

    private static int mainRun(String[] args, OutputStream out, ByteArrayOutputStream err, Main.Workload workload) {
        return Main.run(args, out, new PrintStream(err, true, StandardCharsets.US_ASCII), workload, () -> 0x0123456789abcdefL);
    }

    private static int[] intsFrom(long... values) {
        return Arrays.stream(values).mapToInt(v -> (int) v).toArray();
    }

    public static void main(String[] args) {
        check("mode: buggy, fixed and wide are the three modes", () -> {
            equal(Optional.of(Mode.BUGGY), parse("buggy"));
            equal(Optional.of(Mode.FIXED), parse("fixed"));
            equal(Optional.of(Mode.WIDE), parse("wide"));
        });
        check("mode: nothing, an unknown mode, a wrong-case mode and an extra argument are usage errors", () -> {
            yes(parse().isEmpty(), "no argument");
            yes(parse("racy").isEmpty(), "unknown");
            yes(parse("Buggy").isEmpty(), "wrong case");
            yes(parse("").isEmpty(), "empty");
            yes(parse("buggy", "extra").isEmpty(), "extra argument");
            yes(parse("buggy", "--switch=1.0").isEmpty(), "no second argument exists in Java");
        });
        check("mode: the contract's constants", () -> {
            equal(4, Contract.THREADS);
            equal(1000, Contract.ITERATIONS);
            equal(1000, Contract.WINDOW);
            equal(10000, Contract.WINDOW_WIDE);
            equal(4000, Contract.THREADS * Contract.ITERATIONS);
            equal(5000, Contract.MAX_ITERATIONS);
            equal("java", Contract.LANG);
        });
        check("mode: wide differs from buggy only in the busy loop length, fixed only in the lock", () -> {
            equal(Contract.WINDOW, Mode.BUGGY.window());
            equal(Contract.WINDOW_WIDE, Mode.WIDE.window());
            equal(Contract.WINDOW, Mode.FIXED.window());
            yes(!Mode.BUGGY.locked() && !Mode.WIDE.locked() && Mode.FIXED.locked(), "only fixed is locked");
            equal("wide", Mode.WIDE.wire());
        });
        check("line: the result line has fixed widths and lowercase hex", () -> {
            equal(
                    "LANGAT v1 lang=java mode=buggy final=00000000000000003990 expect=00000000000000004000 "
                            + "order=00000000000000ab rand=0123456789abcdef\n",
                    LangAtLine.format(Mode.BUGGY, 3990, 4000, 0xabL, 0x0123456789abcdefL));
        });
        check("line: the order is the FNV-1a 64 hash of the arrival list written as ASCII digits", () -> {
            equal(0xaf63dc4c8601ec8cL, OrderHash.fnv1a64("a".getBytes(StandardCharsets.US_ASCII)));
            equal(0x85944171f73967e8L, OrderHash.fnv1a64("foobar".getBytes(StandardCharsets.US_ASCII)));
            equal(OrderHash.fnv1a64("2031".getBytes(StandardCharsets.US_ASCII)), OrderHash.of(intsFrom(2, 0, 3, 1)));
            yes(OrderHash.of(intsFrom(0, 1, 2, 3)) != OrderHash.of(intsFrom(3, 2, 1, 0)), "order matters");
        });
        check("barrier: nobody passes until every party has arrived", () -> {
            AtomicInteger started = new AtomicInteger();
            StartBarrier barrier = new StartBarrier(started, 4);
            AtomicInteger passedEarly = new AtomicInteger();
            List<Thread> threads = new ArrayList<>();
            for (int i = 0; i < 4; i++) {
                Thread t = new Thread(() -> {
                    started.incrementAndGet();
                    boolean released = barrier.await();
                    if (!released || started.get() < 4) {
                        passedEarly.incrementAndGet();
                    }
                });
                threads.add(t);
                t.start();
                if (i < 3) {
                    Thread.sleep(20);
                    equal(0, passedEarly.get());
                }
            }
            for (Thread t : threads) {
                t.join();
            }
            equal(0, passedEarly.get());
        });
        check("barrier: an abort wakes the waiters and tells them the start failed", () -> {
            StartBarrier barrier = new StartBarrier(new AtomicInteger(), 4);
            AtomicInteger released = new AtomicInteger(-1);
            Thread waiter = new Thread(() -> released.set(barrier.await() ? 1 : 0));
            waiter.start();
            Thread.sleep(20);
            barrier.abort();
            waiter.join();
            equal(0, released.get());
            yes(!barrier.await(), "a later wait fails too");
        });
        check("race: spin is a pure function of its argument", () -> {
            equal(Race.spin(1000), Race.spin(1000));
            yes(Race.spin(1000) != Race.spin(1001), "different rounds, different value");
            yes(Race.spin(0) != Race.spin(1), "the accumulator moves");
        });
        check("race: spin computes the rotate-multiply sequence, with the rotation written out", () -> {
            for (int rounds : new int[] {0, 1, 2, 7, 64, 1000}) {
                long acc = 0xcbf29ce484222325L;
                for (int i = 0; i < rounds; i++) {
                    acc = (Long.rotateLeft(acc, 7) ^ i) * 0x5851f42d4c957f2dL;
                }
                equal(acc, Race.spin(rounds));
            }
        });
        check("race: the locked mode never loses an update and every worker arrives once", () -> {
            for (int run = 0; run < 20; run++) {
                RaceResult result = Race.run(Mode.FIXED, 200);
                equal(4 * 200L, result.finalValue());
                equal(4, result.started());
                int[] sorted = result.arrival().clone();
                Arrays.sort(sorted);
                equal(new int[] {0, 1, 2, 3}, sorted);
            }
        });
        check("race: the unlocked mode never gains an update", () -> {
            for (int run = 0; run < 20; run++) {
                RaceResult result = Race.run(Mode.BUGGY, 200);
                yes(result.finalValue() <= 4 * 200L, "final " + result.finalValue());
                // Lost updates can pull the total below one thread's own count, but never to nothing.
                yes(result.finalValue() >= 1L, "some update survives: " + result.finalValue());
                equal(4, result.arrival().length);
            }
        });
        check("args: no --k means the contract's 1,000 iterations", () -> {
            equal(Contract.ITERATIONS, invoke("buggy").iterations());
            equal(Mode.BUGGY, invoke("buggy").mode());
            equal(Mode.WIDE, invoke("wide").mode());
        });
        check("args: --k=N as the second argument sets the iterations per worker", () -> {
            equal(50, invoke("buggy", "--k=50").iterations());
            equal(50, invoke("fixed", "--k=50").iterations());
            equal(Mode.FIXED, invoke("fixed", "--k=50").mode());
            equal(1, invoke("buggy", "--k=1").iterations());
            equal(Contract.MAX_ITERATIONS, invoke("buggy", "--k=" + Contract.MAX_ITERATIONS).iterations());
            equal(1000, invoke("buggy", "--k=1000").iterations());
        });
        check("args: --k leaves the threads and the window alone, wide included", () -> {
            equal(Mode.WIDE, invoke("wide", "--k=50").mode());
            equal(Contract.WINDOW_WIDE, invoke("wide", "--k=50").mode().window());
            equal(Contract.THREADS, 4);
        });
        check("args: an invalid --k is a usage error with a message that names the value", () -> {
            for (String bad : new String[] {
                "--k=", "--k=abc", "--k=1.5", "--k=0", "--k=-5", "--k=+5", "--k=050", "--k= 5", "--k=5 ", "--k=0x10",
                "--k=" + (Contract.MAX_ITERATIONS + 1), "--k=99999999999999999999", "--k=2147483648", "--k=\u0665"
            }) {
                Invocation.Parsed parsed = invoke("buggy", bad);
                yes(parsed.error().isPresent(), "refused: " + bad);
                yes(parsed.error().get().startsWith("--k must be"), "reason names --k: " + parsed.error().get());
            }
        });
        check("args: the message does not echo control characters and shortens a long value", () -> {
            String reason = invoke("buggy", "--k=a\u0007b\n\u001b[31m").error().get();
            yes(reason.chars().allMatch(c -> c >= ' ' && c <= '~'), "printable only: " + reason);
            yes(reason.contains("a?b??[31m"), "replaced: " + reason);
            String longReason = invoke("buggy", "--k=" + "9".repeat(500)).error().get();
            yes(longReason.length() < 100, "short: " + longReason.length());
            yes(invoke("racy\u0000").error().get().chars().allMatch(c -> c >= ' ' && c <= '~'), "mode echo is printable");
        });
        check("args: other second arguments, a misspelt option and a third argument are usage errors", () -> {
            for (String[] bad : new String[][] {
                {"buggy", "--switch=1.0"}, {"buggy", "-k=50"}, {"buggy", "--K=50"}, {"buggy", "k=50"}, {"buggy", "--k"},
                {"buggy", "--k=50", "extra"}, {"buggy", "--k=50", "--k=50"}, {"--k=50"}, {"--k=50", "buggy"}, {}, {"racy", "--k=50"}
            }) {
                yes(Invocation.parse(bad).error().isPresent(), "refused: " + Arrays.toString(bad));
            }
        });
        check("main: no --k runs the workload at 1,000 iterations and writes the same line as before", () -> {
            Recording out = new Recording();
            ByteArrayOutputStream err = new ByteArrayOutputStream();
            AtomicInteger seen = new AtomicInteger();
            Main.Workload workload = (mode, k) -> {
                seen.set(k);
                return new RaceResult(3990, 4, intsFrom(2, 0, 3, 1));
            };
            equal(0, mainRun(new String[] {"buggy"}, out, err, workload));
            equal(1000, seen.get());
            equal(
                    "LANGAT v1 lang=java mode=buggy final=00000000000000003990 expect=00000000000000004000 order="
                            + Fmt16.hex(OrderHash.of(intsFrom(2, 0, 3, 1))) + " rand=0123456789abcdef\n",
                    out.text());
        });
        check("main: --k=50 runs the workload at 50 iterations; the line keeps its format, expect is T*K", () -> {
            Recording out = new Recording();
            ByteArrayOutputStream err = new ByteArrayOutputStream();
            AtomicInteger seen = new AtomicInteger();
            Main.Workload workload = (mode, k) -> {
                seen.set(k);
                return new RaceResult(197, 4, intsFrom(2, 0, 3, 1));
            };
            equal(0, mainRun(new String[] {"buggy", "--k=50"}, out, err, workload));
            equal(50, seen.get());
            equal(1, out.writes.size());
            equal(
                    "LANGAT v1 lang=java mode=buggy final=00000000000000000197 expect=00000000000000000200 order="
                            + Fmt16.hex(OrderHash.of(intsFrom(2, 0, 3, 1))) + " rand=0123456789abcdef\n",
                    out.text());
            equal("", err.toString(StandardCharsets.US_ASCII));
        });
        check("main: an invalid --k exits 64 with a message on stderr, nothing on stdout, and never runs the workload", () -> {
            for (String bad : new String[] {"--k=0", "--k=abc", "--k=-1", "--k=" + (Contract.MAX_ITERATIONS + 1)}) {
                Recording out = new Recording();
                ByteArrayOutputStream err = new ByteArrayOutputStream();
                Main.Workload never = (mode, k) -> {
                    throw new AssertionError("the workload must not run");
                };
                equal(64, mainRun(new String[] {"buggy", bad}, out, err, never));
                equal(0, out.writes.size());
                String message = err.toString(StandardCharsets.US_ASCII);
                yes(message.contains("usage error: --k must be an integer from 1 to 5000"), "reason line: " + message);
                yes(message.contains("usage: java"), "usage line: " + message);
            }
        });
        check("race: a short run at K iterations ends at T*K when locked", () -> {
            equal(4 * 50L, Race.run(Mode.FIXED, 50).finalValue());
            equal(4 * 1L, Race.run(Mode.FIXED, 1).finalValue());
        });
        check("main: a good run writes the contract line with one write call and exits 0", () -> {
            Recording out = new Recording();
            ByteArrayOutputStream err = new ByteArrayOutputStream();
            Main.Workload workload = (mode, k) -> new RaceResult(3990, 4, intsFrom(2, 0, 3, 1));
            equal(0, mainRun(new String[] {"buggy"}, out, err, workload));
            equal(1, out.writes.size());
            equal(
                    "LANGAT v1 lang=java mode=buggy final=00000000000000003990 expect=00000000000000004000 order="
                            + Fmt16.hex(OrderHash.of(intsFrom(2, 0, 3, 1))) + " rand=0123456789abcdef\n",
                    out.text());
            equal("", err.toString(StandardCharsets.US_ASCII));
        });
        check("main: a usage error prints a usage line on stderr, nothing on stdout, and exits 64", () -> {
            Recording out = new Recording();
            ByteArrayOutputStream err = new ByteArrayOutputStream();
            Main.Workload never = (mode, k) -> {
                throw new AssertionError("the workload must not run");
            };
            equal(64, mainRun(new String[] {"racy"}, out, err, never));
            equal(0, out.writes.size());
            yes(err.toString(StandardCharsets.US_ASCII).contains("usage"), "usage line");
        });
        check("main: a worker that failed is an invariant failure, exit 3, nothing on stdout", () -> {
            Recording out = new Recording();
            ByteArrayOutputStream err = new ByteArrayOutputStream();
            Main.Workload failing = (mode, k) -> {
                throw new RaceFailure("a worker thread failed to start or died");
            };
            equal(3, mainRun(new String[] {"fixed"}, out, err, failing));
            equal(0, out.writes.size());
            yes(err.toString(StandardCharsets.US_ASCII).contains("a worker thread failed"), "reason");
        });
        check("main: an arrival list that is not four entries long is an invariant failure", () -> {
            Recording out = new Recording();
            ByteArrayOutputStream err = new ByteArrayOutputStream();
            Main.Workload short3 = (mode, k) -> new RaceResult(4000, 4, intsFrom(0, 1, 2));
            equal(3, mainRun(new String[] {"fixed"}, out, err, short3));
            equal(0, out.writes.size());
        });
        check("main: a stdout that cannot be written is an invariant failure, exit 3", () -> {
            ByteArrayOutputStream err = new ByteArrayOutputStream();
            Main.Workload workload = (mode, k) -> new RaceResult(4000, 4, intsFrom(0, 1, 2, 3));
            equal(3, mainRun(new String[] {"fixed"}, new Unwritable(), err, workload));
            yes(err.toString(StandardCharsets.US_ASCII).contains("could not be written"), "reason");
        });
        System.out.println(total - failed + " passed, " + failed + " failed");
        System.exit(failed == 0 ? 0 : 1);
    }

    /** Sixteen lowercase hex digits, written out here so the test does not lean on the code it tests. */
    private static final class Fmt16 {
        static String hex(long value) {
            StringBuilder out = new StringBuilder();
            for (int shift = 60; shift >= 0; shift -= 4) {
                out.append("0123456789abcdef".charAt((int) ((value >>> shift) & 0xf)));
            }
            return out.toString();
        }
    }
}
