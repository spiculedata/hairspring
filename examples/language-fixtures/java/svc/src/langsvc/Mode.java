package langsvc;

/** The three builds of the service, chosen by the first argument: the racy claim, the atomic claim, the racy claim with a wide window. */
enum Mode {
    BUGGY("buggy", Contract.WINDOW, false),
    FIXED("fixed", Contract.WINDOW, true),
    WIDE("wide", Contract.WINDOW_WIDE, false);

    private final String label;
    private final int window;
    private final boolean atomicClaim;

    Mode(String label, int window, boolean atomicClaim) {
        this.label = label;
        this.window = window;
        this.atomicClaim = atomicClaim;
    }

    String label() {
        return label;
    }

    int window() {
        return window;
    }

    boolean atomicClaim() {
        return atomicClaim;
    }

    static Mode parse(String text) {
        for (Mode mode : values()) {
            if (mode.label.equals(text)) {
                return mode;
            }
        }
        return null;
    }
}
