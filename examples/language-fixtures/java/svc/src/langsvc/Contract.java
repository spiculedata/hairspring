package langsvc;

/**
 * The service fixture's constants. Four clients each send K transfers; every transfer is delivered at least twice (the
 * client transport sends a copy and an immediate retransmit); a pool of three workers applies them through an idempotency
 * table. The property: the ledger applied exactly CLIENTS x K transfers.
 */
final class Contract {
    static final String LANG = "java";
    static final int CLIENTS = 4;
    static final int WORKERS = 3;
    static final int ACCOUNTS = 8;
    static final int ITERATIONS = 1000;
    /** The most iterations per client {@code --k} accepts. */
    static final int MAX_ITERATIONS = 5000;
    static final int WINDOW = 1000;
    static final int WINDOW_WIDE = 10000;
    /** Copies sent per transfer before any reply: the original and one immediate retransmit. */
    static final int COPIES = 2;
    /** How long a client waits for a reply before it sends another copy (virtual time under Hermit). */
    static final long REPLY_TIMEOUT_MS = 50;
    /** A transfer's time to live at its first attempt; doubled at each retry, so a retry always gets through in the end. */
    static final long TTL_MS = 2000;
    /** The most copies a client sends for one transfer before it gives up (an invariant failure, exit 3). */
    static final int MAX_ATTEMPTS = 20;
    /** The metrics ticker's period. */
    static final long TICK_MS = 20;
    /** How long shutdown waits for the pool and the ticker in one step. */
    static final long DRAIN_STEP_MS = 100;
    /** The most drain steps before shutdown gives up (an invariant failure, exit 3). */
    static final int DRAIN_STEPS = 600;

    static final int EXIT_OK = 0;
    static final int EXIT_INVARIANT = 3;
    static final int EXIT_USAGE = 64;

    private Contract() {}
}
