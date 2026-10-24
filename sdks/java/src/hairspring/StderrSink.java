package hairspring;

import java.io.IOException;
import java.io.OutputStream;
import java.nio.channels.FileChannel;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.util.List;
import java.util.Set;

/**
 * The report's own channel to standard error. {@code System.err.close()} (or {@code FileDescriptor.err.close()}) makes the
 * JDK point descriptor 2 at {@code /dev/null}, so a report written to descriptor 2 at exit would vanish silently. The JVM
 * cannot duplicate a descriptor, so at start the SDK opens descriptor 2 again through the process's own descriptor
 * directory ({@code /proc/self/fd/2} on Linux, {@code /dev/fd/2} elsewhere) in append mode, a second descriptor of its own
 * that nothing the program does to descriptor 2 can take away. When neither path can be opened (a socket, no such
 * directory) the sink falls back to the stream on descriptor 2.
 *
 * <p>The second descriptor is a new open file description: on a pipe or a terminal it is the same stream, on a regular
 * file it appends at the end of the file instead of sharing the program's write offset.
 */
final class StderrSink {
    static final List<Path> OWN_PATHS = List.of(Path.of("/proc/self/fd/2"), Path.of("/dev/fd/2"));

    private StderrSink() {}

    static Sink open(List<Path> candidates, OutputStream descriptor2) {
        for (Path path : candidates) {
            try {
                return new ChannelSink(FileChannel.open(path, Set.of(StandardOpenOption.WRITE, StandardOpenOption.APPEND)));
            } catch (IOException | RuntimeException unusable) {
                // Try the next path; the last resort is the stream on descriptor 2.
            }
        }
        return new StreamSink(descriptor2);
    }
}
