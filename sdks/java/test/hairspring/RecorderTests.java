package hairspring;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.atomic.AtomicBoolean;

/** The recorder: counting under contention, consistent snapshots, and the report it writes. */
final class RecorderTests {
    private static final long OWN_PID = ProcessHandle.current().pid();

    private RecorderTests() {}

    private static Recorder recorder(Sink sink) {
        return new Recorder(new Identity("p1", true, false), sink, 0, OWN_PID);
    }

    private static String w(long n) {
        return String.format("%20d", n);
    }

    private static void basic(Recorder r) {
        r.declare(Kind.REACHABLE, "basic.reach.never", "never reached");
        r.declare(Kind.UNREACHABLE, "basic.unreach.clean", "never reached either");
        r.declare(Kind.ALWAYS_OR_UNREACHABLE, "basic.aou.never", "never evaluated");
        for (int i = 0; i < 5; i++) {
            r.record(Kind.ALWAYS, true, "basic.always.ok", "always holds", new long[] {i, 2});
        }
        r.record(Kind.ALWAYS, false, "basic.always.bad", "always violated", new long[] {7, 8});
        for (int i = 0; i < 4; i++) {
            r.record(Kind.SOMETIMES, i % 2 == 0, "basic.sometimes", "sometimes holds", new long[] {i});
        }
        r.record(Kind.REACHABLE, true, "basic.reach.hit", "is reached", new long[0]);
        r.work(300);
    }

    private static String assertFor(ListSink sink, String id) {
        return sink.ofKind("assert").stream().filter(l -> l.contains("\"id\":\"" + id + "\"")).findFirst().orElseThrow();
    }

    private static void inThreads(int count, Check.Case... bodies) throws Exception {
        List<Thread> threads = new ArrayList<>();
        CountDownLatch go = new CountDownLatch(1);
        List<Throwable> failures = java.util.Collections.synchronizedList(new ArrayList<>());
        for (int i = 0; i < count; i++) {
            Check.Case body = bodies[i % bodies.length];
            Thread t = new Thread(() -> {
                try {
                    go.await();
                    body.run();
                } catch (Throwable problem) {
                    failures.add(problem);
                }
            });
            threads.add(t);
            t.start();
        }
        go.countDown();
        for (Thread t : threads) {
            t.join();
        }
        if (!failures.isEmpty()) {
            throw new AssertionError("a thread failed: " + failures.get(0), failures.get(0));
        }
    }


    /** A sink whose first line blocks until released, so a report can be caught half written. */
    private static final class BlockingSink implements Sink {
        final ListSink inner = new ListSink();
        final CountDownLatch entered = new CountDownLatch(1);
        final CountDownLatch release = new CountDownLatch(1);

        @Override
        public boolean writeLine(byte[] line) {
            entered.countDown();
            try {
                release.await();
            } catch (InterruptedException interrupted) {
                Thread.currentThread().interrupt();
            }
            return inner.writeLine(line);
        }

        @Override
        public void close() {}
    }

    static void register(Suite s) {
        s.add("recorder: the report has hello, the catalog, the asserts, work, thread lines and end, in that order", () -> {
            ListSink sink = new ListSink();
            Recorder r = recorder(sink);
            basic(r);
            Check.yes(r.report(EndReason.EXIT, null), "written");
            List<String> want = new ArrayList<>(List.of("hello"));
            want.addAll(java.util.Collections.nCopies(7, "catalog"));
            want.addAll(java.util.Collections.nCopies(7, "assert"));
            want.addAll(List.of("work", "thread", "end"));
            Check.equal(want, sink.kinds());
            String end = sink.ofKind("end").get(0);
            Check.yes(end.contains("\"sites\":7,\"evals\":" + w(11) + ",\"dropped\":" + w(0) + ",\"threads\":1}"), end);
            Check.yes(sink.ofKind("work").get(0).contains("\"ops\":" + w(300)), "work");
        });
        s.add("recorder: a witness carries the thread's key, its own ordinal and the values", () -> {
            ListSink sink = new ListSink();
            Recorder r = recorder(sink);
            basic(r);
            r.report(EndReason.EXIT, null);
            String bad = assertFor(sink, "basic.always.bad");
            Check.yes(bad.contains("\"first_false\":{\"thread\":\"t#1\",\"ord\":" + w(6) + ",\"vals\":[7,8,0,0]}"), bad);
            String ok = assertFor(sink, "basic.always.ok");
            Check.yes(ok.contains("\"first_true\":{\"thread\":\"t#1\",\"ord\":" + w(1) + ",\"vals\":[0,2,0,0]}"), ok);
        });
        s.add("recorder: the same id from two places is one site with summed counts", () -> {
            ListSink sink = new ListSink();
            Recorder r = recorder(sink);
            r.record(Kind.SOMETIMES, true, "merge.shared", "one id, two sites", new long[] {1});
            r.record(Kind.SOMETIMES, false, "merge.shared", "one id, two sites", new long[] {1});
            r.record(Kind.SOMETIMES, true, "merge.shared", "one id, two sites", new long[] {2});
            r.record(Kind.SOMETIMES, true, "merge.shared", "one id, two sites", new long[] {2});
            r.report(EndReason.EXIT, null);
            Check.equal(1, sink.ofKind("catalog").size());
            Check.equal(1, sink.ofKind("assert").size());
            Check.yes(sink.ofKind("assert").get(0).contains("\"hits\":" + w(4) + ",\"trues\":" + w(3) + ",\"falses\":" + w(1)), "counts");
            Check.yes(sink.ofKind("end").get(0).contains("\"sites\":1,"), "one site");
        });
        s.add("recorder: an id used with two kinds is a catalog conflict the engine reports", () -> {
            ListSink sink = new ListSink();
            Recorder r = recorder(sink);
            r.record(Kind.ALWAYS, true, "conflict.id", "declared as always", new long[0]);
            r.record(Kind.SOMETIMES, true, "conflict.id", "declared as sometimes", new long[0]);
            r.report(EndReason.EXIT, null);
            Check.equal(2, sink.ofKind("catalog").size());
            Check.equal(1, sink.ofKind("assert").size());
            Check.yes(sink.ofKind("assert").get(0).contains("\"hits\":" + w(1)), "the first kind's counts");
            Check.yes(sink.ofKind("end").get(0).contains("\"sites\":1,"), "one distinct id");
        });
        s.add("recorder: a declared site that never runs is reported with zero hits", () -> {
            ListSink sink = new ListSink();
            Recorder r = recorder(sink);
            r.declare(Kind.REACHABLE, "wal.reset.reached", "the WAL was reset");
            r.report(EndReason.EXIT, null);
            String line = assertFor(sink, "wal.reset.reached");
            Check.yes(line.contains("\"hits\":" + w(0) + ",\"trues\":" + w(0) + ",\"falses\":" + w(0)), line);
            Check.yes(line.contains("\"first_true\":null"), line);
        });
        s.add("recorder: evaluating a declared site is the same site", () -> {
            ListSink sink = new ListSink();
            Recorder r = recorder(sink);
            r.declare(Kind.REACHABLE, "x", "m");
            r.record(Kind.REACHABLE, true, "x", "m", new long[0]);
            r.report(EndReason.EXIT, null);
            Check.equal(1, sink.ofKind("catalog").size());
            Check.yes(assertFor(sink, "x").contains("\"hits\":" + w(1)), "one hit");
        });
        s.add("recorder: a bad id or message or a fifth value is refused and counts nothing", () -> {
            ListSink sink = new ListSink();
            Recorder r = recorder(sink);
            Check.raises(IllegalArgumentException.class, () -> r.record(Kind.ALWAYS, true, "", "m", new long[0]));
            Check.raises(IllegalArgumentException.class, () -> r.record(Kind.ALWAYS, true, "x", "m".repeat(513), new long[0]));
            Check.raises(IllegalArgumentException.class, () -> r.record(Kind.ALWAYS, true, "x", "m", new long[5]));
            Check.raises(IllegalArgumentException.class, () -> r.declare(Kind.ALWAYS, "a\0b", ""));
            Check.raises(IllegalArgumentException.class, () -> r.work(-1));
            r.report(EndReason.EXIT, null);
            Check.equal(0, sink.ofKind("catalog").size());
            Check.yes(sink.ofKind("end").get(0).contains("\"evals\":" + w(0)), "nothing counted");
        });
        s.add("recorder: a bad message is refused on every call, also for a site that is already known", () -> {
            ListSink sink = new ListSink();
            Recorder r = recorder(sink);
            r.record(Kind.ALWAYS, true, "x", "fine", new long[0]);
            Check.raises(IllegalArgumentException.class, () -> r.record(Kind.ALWAYS, true, "x", "m".repeat(513), new long[0]));
            Check.raises(IllegalArgumentException.class, () -> r.declare(Kind.ALWAYS, "x", "m".repeat(513)));
            Check.raises(NullPointerException.class, () -> r.record(Kind.ALWAYS, true, "x", null, new long[0]));
            r.record(Kind.ALWAYS, true, "x", "\u20ac".repeat(170), new long[0]); // 510 bytes: fits
            Check.raises(IllegalArgumentException.class, () -> r.record(Kind.ALWAYS, true, "x", "\u20ac".repeat(171), new long[0]));
            r.report(EndReason.EXIT, null);
            Check.yes(sink.ofKind("end").get(0).contains("\"evals\":" + w(2)), "only the two good calls counted");
        });
        s.add("recorder: the exit hook waits for a report another thread is still writing", () -> {
            BlockingSink sink = new BlockingSink();
            Recorder r = recorder(sink);
            r.record(Kind.REACHABLE, true, "x", "m", new long[0]);
            Thread explicit = new Thread(() -> r.report(EndReason.EXPLICIT, null));
            explicit.start();
            Check.yes(sink.entered.await(10, java.util.concurrent.TimeUnit.SECONDS), "the writer started");
            AtomicBoolean hookReturned = new AtomicBoolean();
            Thread hook = new Thread(() -> {
                r.report(EndReason.EXIT, null);
                hookReturned.set(true);
            });
            hook.start();
            Thread.sleep(300);
            Check.no(hookReturned.get(), "the hook returned while the report was half written");
            sink.release.countDown();
            hook.join(2_000); // woken by the writer finishing, not by the 5 second bound
            Check.yes(hookReturned.get(), "the hook returned promptly once the report was done");
            explicit.join(10_000);
            Check.equal(1, sink.inner.ofKind("end").size());
        });
        s.add("recorder: ids and messages at their limits are accepted", () -> {
            ListSink sink = new ListSink();
            Recorder r = recorder(sink);
            r.record(Kind.REACHABLE, true, "x".repeat(128), "m".repeat(512), new long[0]);
            r.record(Kind.REACHABLE, true, "bad id$with:odd/chars", "quote \" and backslash \\ in the message", new long[0]);
            r.report(EndReason.EXIT, null);
            Check.yes(sink.ofKind("catalog").get(0).contains("\"id\":\"" + "x".repeat(128) + "\""), "long id");
            Check.yes(sink.ofKind("catalog").get(1).contains("\"id\":\"bad_id_with:odd/chars\""), "sanitised id");
        });
        s.add("recorder: eight contending threads leave exact totals", () -> {
            ListSink sink = new ListSink();
            Recorder r = recorder(sink);
            int each = 20_000;
            Check.Case body = () -> {
                for (int i = 0; i < each; i++) {
                    r.record(Kind.SOMETIMES, i % 2 == 0, "contended", "m", new long[] {i});
                }
            };
            inThreads(8, body);
            r.report(EndReason.EXIT, null);
            String line = assertFor(sink, "contended");
            Check.yes(line.contains("\"hits\":" + w(8L * each) + ",\"trues\":" + w(4L * each) + ",\"falses\":" + w(4L * each)), line);
            Check.equal(8, sink.ofKind("thread").size());
            long evals = sink.ofKind("thread").stream()
                    .mapToLong(l -> Long.parseLong(l.substring(l.indexOf("\"evals\":") + 8, l.indexOf(",\"tape")).trim()))
                    .sum();
            Check.equal(8L * each, evals);
        });
        s.add("recorder: a snapshot taken while threads evaluate is consistent", () -> {
            Recorder r = recorder(new ListSink());
            AtomicBoolean stop = new AtomicBoolean();
            List<Thread> threads = new ArrayList<>();
            for (int t = 0; t < 4; t++) {
                Thread thread = new Thread(() -> {
                    long i = 0;
                    while (!stop.get()) {
                        r.record(Kind.ALWAYS, i % 3 != 0, "live", "m", new long[] {i});
                        i++;
                    }
                });
                threads.add(thread);
                thread.start();
            }
            try {
                for (int n = 0; n < 3000; n++) {
                    Snapshot snap = r.snapshot();
                    long hits = 0;
                    for (Rows.AssertRow row : snap.asserts()) {
                        hits += row.trues() + row.falses();
                        Check.yes((row.firstTrue() != null) == (row.trues() > 0),
                                "a true witness appears exactly when trues is non-zero");
                        Check.yes(row.falses() > 0 || row.firstFalse() == null, "no false witness behind a zero count");
                    }
                    Check.equal("the evaluations of the thread lines equal the hits of the assert lines", snap.evals(), hits);
                }
            } finally {
                stop.set(true);
                for (Thread thread : threads) {
                    thread.join();
                }
            }
        });
        s.add("recorder: the report is written once and later evaluations are not reported", () -> {
            ListSink sink = new ListSink();
            Recorder r = recorder(sink);
            r.record(Kind.REACHABLE, true, "explicit.before", "m", new long[0]);
            Check.yes(r.report(EndReason.EXPLICIT, null), "first");
            int written = sink.lines().size();
            r.record(Kind.REACHABLE, true, "explicit.after", "m", new long[0]);
            Check.no(r.report(EndReason.EXIT, null), "second");
            Check.equal(written, sink.lines().size());
            Check.yes(sink.ofKind("end").get(0).contains("\"reason\":\"explicit\""), "reason");
        });
        s.add("recorder: a process that did not start the SDK writes nothing", () -> {
            ListSink sink = new ListSink();
            Recorder r = new Recorder(new Identity("p1", true, false), sink, 0, OWN_PID + 1);
            r.record(Kind.REACHABLE, true, "x", "m", new long[0]);
            Check.no(r.report(EndReason.EXIT, null), "report");
            Check.equal(0, sink.lines().size());
        });
        s.add("recorder: with the output off the program counts as usual and writes nothing", () -> {
            Recorder r = new Recorder(new Identity("p1", true, false), null, 0, OWN_PID);
            r.record(Kind.REACHABLE, true, "x", "m", new long[0]);
            Check.no(r.report(EndReason.EXIT, null), "report");
        });
        s.add("recorder: threads beyond the table are capped and counted without a failure", () -> {
            ListSink sink = new ListSink();
            Recorder r = recorder(sink);
            for (int i = 0; i < 300; i++) {
                Thread t = new Thread(() -> r.record(Kind.SOMETIMES, true, "threads.each", "m", new long[0]));
                t.start();
                t.join();
            }
            r.report(EndReason.EXIT, null);
            Check.equal(256, sink.ofKind("thread").size());
            String end = sink.ofKind("end").get(0);
            Check.yes(end.contains("\"threads\":256}"), end);
            Check.no(end.contains("\"dropped\":" + w(0)), "dropped");
        });
        s.add("recorder: thread() gives the calling thread a stable key", () -> {
            ListSink sink = new ListSink();
            Recorder r = recorder(sink);
            Check.yes(r.thread("worker", 3), "claimed");
            r.record(Kind.REACHABLE, true, "x", "m", new long[0]);
            r.report(EndReason.EXIT, null);
            String line = sink.ofKind("thread").get(0);
            Check.yes(line.contains("\"thread\":\"worker-3\""), line);
            Check.yes(line.endsWith("\"key_stable\":true }\n"), line);
            Check.yes(assertFor(sink, "x").contains("\"thread\":\"worker-3\""), "the witness names it too");
        });
        s.add("recorder: work counts per thread and is reported as one total", () -> {
            ListSink sink = new ListSink();
            Recorder r = recorder(sink);
            inThreads(4, () -> {
                for (int i = 0; i < 1000; i++) {
                    r.work(1);
                }
            });
            r.report(EndReason.EXIT, null);
            Check.yes(sink.ofKind("work").get(0).contains("\"ops\":" + w(4000)), sink.ofKind("work").get(0));
        });
        s.add("recorder: the caller's file and line are recorded at the first evaluation", () -> {
            ListSink sink = new ListSink();
            Recorder r = recorder(sink);
            r.record(Kind.REACHABLE, true, "where", "m", new long[0]);
            r.report(EndReason.EXIT, null);
            String catalog = sink.ofKind("catalog").get(0);
            Check.yes(catalog.contains("\"loc\":{\"file\":\"hairspring/RecorderTests.java\",\"line\":"), catalog);
            Check.no(catalog.contains("\"line\":0}"), "a line number");
        });
        s.add("recorder: a line the sink refuses is counted in end.dropped, and the end line is still sent", () -> {
            ListSink sink = new ListSink(line -> line.contains("\"kind\":\"work\""));
            Recorder r = recorder(sink);
            r.work(1);
            r.report(EndReason.EXIT, null);
            Check.yes(sink.ofKind("end").get(0).contains("\"dropped\":" + w(1)), sink.ofKind("end").get(0));
        });
        s.add("recorder: the status the program recorded is in the end line", () -> {
            ListSink sink = new ListSink();
            Recorder r = recorder(sink);
            r.setStatus(3);
            r.report(EndReason.EXIT, null);
            Check.yes(sink.ofKind("end").get(0).contains("\"status\":" + w(3)), "status");
        });
        s.add("recorder: a child process is given the tag and the owning pid", () -> {
            Recorder r = recorder(new ListSink());
            Check.equal(Map.of("HAIRSPRING_PROC", "p1", "HAIRSPRING_PROC_PID", Long.toString(OWN_PID)), r.childEnvironment());
            r.report(EndReason.EXIT, null);
            Check.equal(Map.of("HAIRSPRING_PROC", "p1", "HAIRSPRING_PROC_PID", OWN_PID + ":a"), r.childEnvironment());
        });
        s.add("recorder: an unstable root tag is passed on as unstable", () -> {
            Recorder r = new Recorder(new Identity("p1.s9", false, false), new ListSink(), 0, OWN_PID);
            Check.equal(OWN_PID + ":u", r.childEnvironment().get("HAIRSPRING_PROC_PID"));
        });
        s.add("recorder: a refused harness tag is counted and the default tag is used", () -> {
            ListSink sink = new ListSink();
            Recorder r = Recorder.fromEnvironment(Map.of("HAIRSPRING_PROC", "bad tag"), sink, 0, OWN_PID);
            r.report(EndReason.EXIT, null);
            Check.yes(sink.ofKind("hello").get(0).contains("\"proc\":\"p1\""), "default tag");
            Check.yes(sink.ofKind("end").get(0).contains("\"dropped\":" + w(1)), "counted");
        });
        s.add("recorder: a spawned child takes the parent's tag with its own pid and says it is unstable", () -> {
            ListSink sink = new ListSink();
            long parent = OWN_PID + 7;
            Recorder r = Recorder.fromEnvironment(
                    Map.of("HAIRSPRING_PROC", "p1", "HAIRSPRING_PROC_PID", Long.toString(parent)), sink, 0, OWN_PID);
            r.report(EndReason.EXIT, null);
            String hello = sink.ofKind("hello").get(0);
            Check.yes(hello.contains("\"proc\":\"p1.s" + OWN_PID + "\""), hello);
            Check.yes(hello.endsWith("\"tag_stable\":false}\n"), hello);
        });
    }
}
