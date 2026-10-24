package langat;

/** The one stdout line of the program, in the form every language's fixture prints. */
final class LangAtLine {
    private static final int COUNT_WIDTH = 20;
    private static final int HEX_WIDTH = 16;

    private LangAtLine() {}

    private static String zeroPadded(String digits, int width) {
        return "0".repeat(Math.max(0, width - digits.length())) + digits;
    }

    private static String hex(long value) {
        return zeroPadded(Long.toHexString(value), HEX_WIDTH);
    }

    /** {@code LANGAT v1 lang=java mode=<mode> final=<20 digits> expect=<20 digits> order=<16 hex> rand=<16 hex>} and a newline. */
    static String format(Mode mode, long finalValue, long order, long rand) {
        return "LANGAT v1 lang=" + Contract.LANG
                + " mode=" + mode.wire()
                + " final=" + zeroPadded(Long.toString(finalValue), COUNT_WIDTH)
                + " expect=" + zeroPadded(Long.toString(Contract.EXPECTED), COUNT_WIDTH)
                + " order=" + hex(order)
                + " rand=" + hex(rand)
                + "\n";
    }
}
