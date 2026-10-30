package langsvc;

import java.io.ByteArrayOutputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/** Self-test of the service fixture's classes, in one JVM (run with HAIRSPRING_EVENTS=off). Exits 1 on the first failure. */
public final class SvcSelfTest {
    private static final Pattern LINE = Pattern.compile(
            "LANGAT v1 lang=java mode=(buggy|fixed|wide) final=(\\d{20}) expect=(\\d{20}) order=([0-9a-f]{16}) rand=([0-9a-f]{16})\n");
    private static int checks;

    private SvcSelfTest() {}

    public static void main(String[] args) {
        usage();
        codec();
        int fixedFires = runs("fixed", 50, 30);
        check(fixedFires == 0, "fixed fired " + fixedFires + " of 30");
        int wideFires = runs("wide", 50, 30);
        check(wideFires >= 1, "wide fired " + wideFires + " of 30 (want at least 1 on a multi-CPU host)");
        int buggyFires = runs("buggy", 50, 30);
        System.out.println("selftest ok: " + checks + " checks; in-process fires at K=50: fixed 0/30, wide " + wideFires + "/30, buggy "
                + buggyFires + "/30");
    }

    private static void usage() {
        for (String[] bad : new String[][] {{}, {"nope"}, {"buggy", "--k=0"}, {"buggy", "--k=01"}, {"buggy", "--k=5001"},
                {"buggy", "--k=x"}, {"buggy", "--k=50", "extra"}, {"buggy", "-k=5"}}) {
            ByteArrayOutputStream err = new ByteArrayOutputStream();
            int rc = Main.run(bad, new ByteArrayOutputStream(), new PrintStream(err, true, StandardCharsets.UTF_8), () -> 0L);
            check(rc == Contract.EXIT_USAGE, "usage rc for " + String.join(" ", bad) + " was " + rc);
        }
        check(Main.parseK("--k=5000") == 5000 && Main.parseK("--k=1") == 1, "parseK bounds");
    }

    private static void codec() {
        Messages.Transfer t = new Messages.Transfer(Messages.key(3, 49), 1, 7, 5);
        check(Messages.decode(Messages.encode(t)).equals(t), "codec round trip");
        check(Messages.key(1, 0) != Messages.key(0, 1), "keys distinct");
    }

    /** Runs a mode {@code n} times at K and returns how often the property was false (final != expect). */
    private static int runs(String mode, int k, int n) {
        int fires = 0;
        for (int i = 0; i < n; i++) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            int rc = Main.run(new String[] {mode, "--k=" + k}, out, System.err, () -> 0x1234L);
            check(rc == Contract.EXIT_OK, mode + " rc " + rc);
            Matcher m = LINE.matcher(out.toString(StandardCharsets.US_ASCII));
            check(m.matches(), mode + " line: " + out);
            long fin = Long.parseLong(m.group(2));
            long exp = Long.parseLong(m.group(3));
            check(exp == 4L * k, mode + " expect " + exp);
            check(fin >= exp, mode + " final below expect (a lost transfer): " + fin);
            if (fin != exp) {
                fires++;
            }
        }
        return fires;
    }

    private static void check(boolean ok, String what) {
        checks++;
        if (!ok) {
            System.out.println("FAIL " + what);
            System.exit(1);
        }
    }
}
