package langat;

/** An internal invariant failed: a thread did not start or died, or the orders do not hold four entries. */
final class InvariantException extends Exception {
    private static final long serialVersionUID = 1L;

    InvariantException(String message) {
        super(message);
    }
}
