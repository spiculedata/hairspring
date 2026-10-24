package hairspring;

import java.io.IOException;
import java.io.OutputStream;
import java.nio.channels.FileChannel;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.List;
import java.util.Set;
import java.util.function.BooleanSupplier;

/**
 * The report's way to standard error. Lines go through descriptor 2, as the program's own writes do, for as long as
 * {@code FileDescriptor.err} is valid. {@code System.err.close()} (or {@code FileDescriptor.err.close()}) makes the JDK
 * point descriptor 2 at {@code /dev/null} and the descriptor object invalid; a report written to descriptor 2 after that
 * would vanish. So at start the SDK also opens descriptor 2 again through the process's own descriptor directory
 * ({@code /proc/self/fd/2} on Linux, {@code /dev/fd/2} elsewhere) in append mode, and switches to that second descriptor
 * only once descriptor 2 has become invalid.
 *
 * <p>The second descriptor is opened only when descriptor 2 is not a regular file. Reopening a path grants the access the
 * path allows, not the access descriptor 2 has: a JVM launched with standard error closed ({@code 2>&-}) gets its first
 * open file, a JDK file opened read-only, as descriptor 2, and reopening that for writing would write the report into the
 * JDK. A regular file is also where a second descriptor does harm: it appends at the end instead of sharing the program's
 * offset, so the program's later writes would overwrite the report. The price is that a program that closes
 * {@code System.err} while standard error is a regular file loses the report. A pipe or a terminal is the same stream
 * through either descriptor.
 */
final class StderrSink implements Sink {
    static final List<Path> OWN_PATHS = List.of(Path.of("/proc/self/fd/2"), Path.of("/dev/fd/2"));

    private final Sink descriptor2;
    private final Sink second;
    private final BooleanSupplier descriptor2IsValid;

    private StderrSink(Sink descriptor2, Sink second, BooleanSupplier descriptor2IsValid) {
        this.descriptor2 = descriptor2;
        this.second = second;
        this.descriptor2IsValid = descriptor2IsValid;
    }

    /**
     * {@code allowRegularFile} is for tests: the real caller passes false. A path that is a regular file, or cannot be
     * opened, is not used; with none left only descriptor 2 is used.
     */
    static Sink open(List<Path> candidates, OutputStream descriptor2, BooleanSupplier descriptor2IsValid, boolean allowRegularFile) {
        Sink primary = new StreamSink(descriptor2);
        for (Path path : candidates) {
            try {
                if (!allowRegularFile && Files.readAttributes(path, BasicFileAttributes.class).isRegularFile()) {
                    return primary;
                }
                FileChannel channel = FileChannel.open(path, Set.of(StandardOpenOption.WRITE, StandardOpenOption.APPEND));
                return new StderrSink(primary, new ChannelSink(channel), descriptor2IsValid);
            } catch (IOException | RuntimeException unusable) {
                // Try the next path.
            }
        }
        return primary;
    }

    @Override
    public boolean writeLine(byte[] line) {
        if (second != null && !descriptor2IsValid.getAsBoolean()) {
            return second.writeLine(line);
        }
        return descriptor2.writeLine(line);
    }

    @Override
    public void close() {
        if (second != null) {
            second.close();
        }
    }
}
