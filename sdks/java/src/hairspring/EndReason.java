package hairspring;

/** Why a process's report ends: normal exit, a fatal signal (never sent by this SDK) or an explicit {@code report()}. */
enum EndReason {
    EXIT("exit"),
    SIGNAL("signal"),
    EXPLICIT("explicit");

    private final String wire;

    EndReason(String wire) {
        this.wire = wire;
    }

    String wire() {
        return wire;
    }
}
