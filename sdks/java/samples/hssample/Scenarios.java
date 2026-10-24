package hssample;

import hairspring.Hairspring;
import hairspring.Kind;
import java.util.ArrayList;
import java.util.List;

/** The scenarios of {@link Sample}: each returns the program's exit status (0 returns from main normally). */
final class Scenarios {
    static final List<String> NAMES = List.of(
            "basic", "merge", "threads", "exit_status", "plain_exit", "raises", "halt", "big", "spawn", "conflict",
            "empty", "limits", "explicit", "daemon", "close_err", "daemon_report");

    private static final String LONG_MSG = "a deliberately long message so that each catalog line is large and two processes "
            + "writing at once have a real chance of interleaving if a line were ever split across write calls; padding "
            + "padding padding padding padding padding padding padding padding padding padding padding padding padding "
            + "padding padding padding padding padding padding padding padding padding padding padding";
    private static final int BIG_SITES = 64;
    private static final int BIG_ROUNDS = 3;
    private static final int ID_LIMIT = 128;
    private static final int MSG_LIMIT = 512;
    private static final int DAEMON_RUN_MILLIS = 50;
    private static final int HALT_STATUS = 5;
    private static final int PLAIN_EXIT_STATUS = 7;
    private static final int CHILD_FAILED = 3;

    private Scenarios() {}

    static int run(String name, String[] args) throws Exception {
        return switch (name) {
            case "basic" -> basic();
            case "merge" -> merge();
            case "threads" -> threads(args.length > 0 ? Integer.parseInt(args[0]) : 0);
            case "exit_status" -> exitStatus();
            case "plain_exit" -> plainExit();
            case "raises" -> raises();
            case "halt" -> halt();
            case "big" -> big();
            case "spawn" -> spawn();
            case "conflict" -> conflict();
            case "empty" -> empty();
            case "limits" -> limits();
            case "explicit" -> explicit();
            case "daemon" -> daemon();
            case "close_err" -> closeErr();
            case "daemon_report" -> daemonReport();
            default -> throw new IllegalArgumentException(name);
        };
    }

    private static int basic() {
        Hairspring.declare(Kind.REACHABLE, "basic.reach.never", "never reached");
        Hairspring.declare(Kind.UNREACHABLE, "basic.unreach.clean", "never reached either");
        Hairspring.declare(Kind.ALWAYS_OR_UNREACHABLE, "basic.aou.never", "never evaluated");
        for (int i = 0; i < 5; i++) {
            Hairspring.always(true, "basic.always.ok", "always holds", i, 2);
        }
        Hairspring.always(false, "basic.always.bad", "always violated", 7, 8);
        for (int i = 0; i < 4; i++) {
            Hairspring.sometimes(i % 2 == 0, "basic.sometimes", "sometimes holds", i);
        }
        Hairspring.reachable("basic.reach.hit", "is reached");
        Hairspring.work(300);
        return 0;
    }

    private static void mergeA(boolean ok) {
        Hairspring.sometimes(ok, "merge.shared", "one id, two sites", 1);
    }

    private static void mergeB(boolean ok) {
        Hairspring.sometimes(ok, "merge.shared", "one id, two sites", 2);
    }

    private static int merge() {
        mergeA(true);
        mergeA(false);
        mergeB(true);
        mergeB(true);
        Hairspring.reachable("merge.other", "a second, distinct id");
        return 0;
    }

    private static int threads(int count) throws InterruptedException {
        Hairspring.work(1);
        for (int i = 0; i < count; i++) {
            int index = i;
            Thread worker = new Thread(() -> {
                Hairspring.thread("w", index);
                Hairspring.sometimes(true, "threads.each", "every thread evaluates", index);
            });
            worker.start();
            worker.join();
        }
        return 0;
    }

    private static int exitStatus() {
        Hairspring.reachable("exit.reach", "reached");
        Hairspring.exit(CHILD_FAILED);
        return 0;
    }

    private static int plainExit() {
        Hairspring.reachable("plain.reach", "reached");
        System.exit(PLAIN_EXIT_STATUS);
        return 0;
    }

    private static int raises() {
        Hairspring.reachable("raises.before", "reached before the exception");
        throw new IllegalStateException("deliberate");
    }

    private static int halt() {
        Hairspring.reachable("halt.before", "reached before the halt");
        Runtime.getRuntime().halt(HALT_STATUS);
        return 0;
    }

    private static int big() {
        for (int round = 0; round < BIG_ROUNDS; round++) {
            for (int site = 0; site < BIG_SITES; site++) {
                Hairspring.sometimes(round % 2 == 0, "big.site." + site, LONG_MSG, round);
            }
        }
        return 0;
    }

    private static int spawn() throws Exception {
        Hairspring.reachable("spawn.parent", "parent reaches this");
        String java = ProcessHandle.current().info().command().orElse("java");
        List<String> command = new ArrayList<>(
                List.of(java, "-cp", System.getProperty("java.class.path"), "hssample.Sample", "merge"));
        Process child = Hairspring.prepare(new ProcessBuilder(command).inheritIO()).start();
        return child.waitFor() == 0 ? 0 : CHILD_FAILED;
    }

    private static int conflict() {
        Hairspring.always(true, "conflict.id", "declared as always");
        Hairspring.sometimes(true, "conflict.id", "declared as sometimes");
        return 0;
    }

    private static int empty() {
        Hairspring.work(1);
        return 0;
    }

    private static int limits() {
        Hairspring.reachable("x".repeat(ID_LIMIT), "m".repeat(MSG_LIMIT));
        Hairspring.reachable("bad id$with:odd/chars", "quote \" and backslash \\ in the message");
        return 0;
    }

    private static int explicit() {
        Hairspring.reachable("explicit.before", "reached before the report");
        Hairspring.report();
        Hairspring.reachable("explicit.after", "reached after the report");
        return 0;
    }

    /** A daemon thread keeps evaluating while the JVM shuts down, so the report is taken against live writers. */
    private static int daemon() throws InterruptedException {
        Thread spinner = new Thread(() -> {
            for (long i = 0; ; i++) {
                Hairspring.sometimes(i % 2 == 0, "daemon.loop", "evaluated while the JVM exits", i);
            }
        });
        spinner.setDaemon(true);
        spinner.start();
        Thread.sleep(DAEMON_RUN_MILLIS);
        return 0;
    }

    /** The program closes System.err (the JDK points descriptor 2 at /dev/null); the report must still arrive. */
    private static int closeErr() {
        Hairspring.reachable("close_err.site", "evaluated before and after the close");
        System.err.close();
        Hairspring.reachable("close_err.after", "evaluated after the close");
        return 0;
    }

    /** A daemon thread reports explicitly while main returns and the JVM exits; the report must still be whole. */
    private static int daemonReport() {
        Hairspring.reachable("daemon_report.site", "reached before the report");
        Thread reporter = new Thread(Hairspring::report);
        reporter.setDaemon(true);
        reporter.start();
        return 0;
    }
}
