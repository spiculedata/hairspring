package hairspring;

/** Where finished protocol lines go. The real ones are {@link StreamSink} and {@link ChannelSink}; the tests use a list. */
interface Sink {
    /** Writes one whole line. False means the line was not (or not completely) written. */
    boolean writeLine(byte[] line);

    /** Releases what the sink owns (a sink on standard error owns nothing). */
    void close();
}
