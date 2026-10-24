package langat;

import java.util.Optional;

/**
 * The three modes of the one program, chosen by the first argument and never by a build flag: {@code buggy} (the planted
 * race), {@code fixed} (the same three steps under a lock) and {@code wide} ({@code buggy} with a busy loop ten times
 * longer).
 */
enum Mode {
    BUGGY("buggy", Contract.WINDOW, false),
    FIXED("fixed", Contract.WINDOW, true),
    WIDE("wide", Contract.WINDOW_WIDE, false);

    private final String wire;
    private final int window;
    private final boolean locked;

    Mode(String wire, int window, boolean locked) {
        this.wire = wire;
        this.window = window;
        this.locked = locked;
    }

    String wire() {
        return wire;
    }

    /** Rounds of the busy loop between the load and the store. */
    int window() {
        return window;
    }

    /** Whether the load, the busy loop and the store sit inside a monitor. */
    boolean locked() {
        return locked;
    }

    /** The mode named by exactly one argument, or empty for a usage error. */
    static Optional<Mode> parse(String[] args) {
        if (args.length != 1) {
            return Optional.empty();
        }
        for (Mode mode : values()) {
            if (mode.wire.equals(args[0])) {
                return Optional.of(mode);
            }
        }
        return Optional.empty();
    }
}
