package hairspring;

/** A line did not fit the protocol's cap; the caller counts it as dropped and never sends a truncated line. */
final class LineTooLongException extends RuntimeException {
    private static final long serialVersionUID = 1L;

    LineTooLongException(String message) {
        super(message);
    }
}
