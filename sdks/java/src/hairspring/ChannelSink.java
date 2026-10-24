package hairspring;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.channels.WritableByteChannel;

/** Writes each line to a channel it owns (the events file, opened for appending). A short write is completed. */
final class ChannelSink implements Sink {
    private final WritableByteChannel channel;

    ChannelSink(WritableByteChannel channel) {
        this.channel = channel;
    }

    @Override
    public boolean writeLine(byte[] line) {
        ByteBuffer buffer = ByteBuffer.wrap(line);
        try {
            while (buffer.hasRemaining()) {
                channel.write(buffer);
            }
            return true;
        } catch (IOException lost) {
            return false;
        }
    }

    @Override
    public void close() {
        try {
            channel.close();
        } catch (IOException ignored) {
            // Nothing is left to write; a failed close loses no line.
        }
    }
}
