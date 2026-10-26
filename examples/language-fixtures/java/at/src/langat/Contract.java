package langat;

/** The constants every language's fixture shares: four workers, a thousand iterations, a busy loop of a thousand rounds. */
final class Contract {
    static final String LANG = "java";
    static final int THREADS = 4;
    static final int ITERATIONS = 1000;
    static final int WINDOW = 1000;
    static final int WINDOW_WIDE = 10000;
    static final int EXPECTED = THREADS * ITERATIONS;
    /** The most iterations per worker {@code --k} accepts (a hundred times the contract's). */
    static final int MAX_ITERATIONS = 100000;

    static final int EXIT_OK = 0;
    static final int EXIT_INVARIANT = 3;
    static final int EXIT_USAGE = 64;

    private Contract() {}
}
