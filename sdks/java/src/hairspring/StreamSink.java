package hairspring;

import java.io.IOException;
import java.io.OutputStream;

/**
 * Writes each line with one call on an unbuffered stream, which for a {@link java.io.FileOutputStream} on a file
 * descriptor is one {@code write(2)}. A reader that has gone away shows as an {@link IOException} (the JVM ignores
 * {@code SIGPIPE}), which costs the line, not the process. The sink does not own the stream and never closes it.
 */
final class StreamSink implements Sink {
    private final OutputStream out;

    StreamSink(OutputStream out) {
        this.out = out;
    }

    @Override
    public boolean writeLine(byte[] line) {
        try {
            out.write(line, 0, line.length);
            out.flush();
            return true;
        } catch (IOException lost) {
            return false;
        }
    }

    @Override
    public void close() {}
}
