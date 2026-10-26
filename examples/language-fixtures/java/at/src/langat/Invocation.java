package langat;

import java.util.Optional;
import java.util.regex.Pattern;

/**
 * The command line: a mode, and optionally {@code --k=N} as the second argument, the iterations per worker. Without it
 * the iterations are the contract's {@link Contract#ITERATIONS}. {@code --k} changes nothing else: the workers, the window
 * and the wide window stay as the contract has them. The value is plain ASCII digits with no sign, no leading zero and
 * no more than {@link Contract#MAX_ITERATIONS}.
 */
record Invocation(Mode mode, int iterations) {
    private static final String K_PREFIX = "--k=";
    private static final int MAX_SHOWN = 24;
    private static final Pattern K_VALUE = Pattern.compile("[1-9][0-9]{0,3}");

    /** What parsing gave: an invocation, or the reason it is a usage error. */
    record Parsed(Optional<Invocation> invocation, Optional<String> error) {
        Mode mode() {
            return invocation.orElseThrow().mode();
        }

        int iterations() {
            return invocation.orElseThrow().iterations();
        }
    }

    private static Parsed refused(String reason) {
        return new Parsed(Optional.empty(), Optional.of(reason));
    }

    static Parsed parse(String[] args) {
        if (args.length < 1 || args.length > 2) {
            return refused("expected a mode and at most one option");
        }
        Optional<Mode> mode = Mode.byName(args[0]);
        if (mode.isEmpty()) {
            return refused("unknown mode \"" + printable(args[0]) + "\"");
        }
        int iterations = Contract.ITERATIONS;
        if (args.length == 2) {
            if (!args[1].startsWith(K_PREFIX)) {
                return refused("unknown option \"" + printable(args[1]) + "\"");
            }
            String value = args[1].substring(K_PREFIX.length());
            Optional<Integer> k = parseK(value);
            if (k.isEmpty()) {
                return refused("--k must be an integer from 1 to " + Contract.MAX_ITERATIONS + ", got \"" + printable(value) + "\"");
            }
            iterations = k.get();
        }
        return new Parsed(Optional.of(new Invocation(mode.get(), iterations)), Optional.empty());
    }

    /** The text as a message may show it: at most 24 characters, anything but printable ASCII shown as '?'. */
    private static String printable(String text) {
        StringBuilder shown = new StringBuilder();
        for (int i = 0; i < text.length() && i < MAX_SHOWN; i++) {
            char c = text.charAt(i);
            shown.append(c >= ' ' && c <= '~' ? c : '?');
        }
        return text.length() > MAX_SHOWN ? shown + "..." : shown.toString();
    }

    private static Optional<Integer> parseK(String value) {
        if (!K_VALUE.matcher(value).matches()) {
            return Optional.empty();
        }
        int k = Integer.parseInt(value);
        return k <= Contract.MAX_ITERATIONS ? Optional.of(k) : Optional.empty();
    }
}
