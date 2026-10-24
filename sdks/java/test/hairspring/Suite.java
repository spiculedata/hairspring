package hairspring;

import java.util.ArrayList;
import java.util.List;

/** A list of named test cases and a runner that prints one line per case. */
final class Suite {
    private record Named(String name, Check.Case body) {}

    private final List<Named> cases = new ArrayList<>();

    void add(String name, Check.Case body) {
        cases.add(new Named(name, body));
    }

    int size() {
        return cases.size();
    }

    /** Runs every case; returns how many failed. */
    int run(java.io.PrintStream out) {
        int failed = 0;
        for (Named one : cases) {
            try {
                one.body().run();
                out.println("ok   " + one.name());
            } catch (Throwable problem) {
                failed++;
                out.println("FAIL " + one.name() + ": " + problem);
                if (!(problem instanceof AssertionError)) {
                    problem.printStackTrace(out);
                }
            }
        }
        out.println(cases.size() - failed + " passed, " + failed + " failed");
        return failed;
    }
}
