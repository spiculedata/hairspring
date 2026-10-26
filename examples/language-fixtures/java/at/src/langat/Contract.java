package langat;

/** The constants every language's fixture shares: four workers, a thousand iterations, a busy loop of a thousand rounds. */
final class Contract {
    static final String LANG = "java";
    static final int THREADS = 4;
    static final int ITERATIONS = 1000;
    static final int WINDOW = 1000;
    static final int WINDOW_WIDE = 10000;
    /**
     * The most iterations per worker {@code --k} accepts. Race.java explains why it is bounded: above some K the JIT
     * compiles the worker loop and the window can vanish. 5,000 was measured clean (PrintCompilation, default flags, the
     * two profiles and -XX:-TieredCompilation); 10,000 to 20,000 was not, with some flags.
     */
    static final int MAX_ITERATIONS = 5000;

    static final int EXIT_OK = 0;
    static final int EXIT_INVARIANT = 3;
    static final int EXIT_USAGE = 64;

    private Contract() {}
}
