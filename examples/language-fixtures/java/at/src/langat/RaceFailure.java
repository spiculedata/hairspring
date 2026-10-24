package langat;

/** An internal invariant of the program failed (a worker did not start or died, the barrier broke): exit status 3. */
final class RaceFailure extends Exception {
    private static final long serialVersionUID = 1L;

    RaceFailure(String message) {
        super(message);
    }
}
