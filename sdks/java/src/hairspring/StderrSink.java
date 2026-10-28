package hairspring;

import java.io.IOException;
import java.io.OutputStream;
import java.nio.channels.FileChannel;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.nio.charset.StandardCharsets;
import java.nio.file.attribute.BasicFileAttributes;
import java.util.List;
import java.util.Set;
import java.util.function.BooleanSupplier;
import java.util.function.Supplier;

/**
 * The report's way to standard error. Lines go through descriptor 2, as the program's own writes do, for as long as
 * {@code FileDescriptor.err} is valid. {@code System.err.close()} (or {@code FileDescriptor.err.close()}) makes the JDK
 * point descriptor 2 at {@code /dev/null} and the descriptor object invalid; a report written to descriptor 2 after that
 * would vanish. So at start the SDK also opens descriptor 2 again through the process's own descriptor directory
 * ({@code /proc/self/fd/2} on Linux, {@code /dev/fd/2} elsewhere) in append mode, and switches to that second descriptor
 * only once descriptor 2 has become invalid.
 *
 * <p>The second descriptor is opened for a regular file only when descriptor 2 was opened for writing, which Linux shows in
 * {@code /proc/self/fdinfo/2} (the access mode in the octal {@code flags} line). Reopening a path grants the access the
 * path allows, not the access descriptor 2 has: a JVM launched with standard error closed ({@code 2>&-}) gets its first
 * open file, a JDK file opened read-only, as descriptor 2, and reopening that for writing would write the report into the
 * JDK. Where fdinfo cannot be read (macOS has no {@code /proc}) or does not say the descriptor can be written, a regular
 * file is not reopened, and a program that closes {@code System.err} there loses the report. A pipe or a terminal is the
 * same stream through either descriptor and is reopened without the check.
 */
final class StderrSink implements Sink {
    static final List<Path> OWN_PATHS = List.of(Path.of("/proc/self/fd/2"), Path.of("/dev/fd/2"));

    /** Linux's account of descriptor 2, including the flags it was opened with. */
    static final Path OWN_FDINFO = Path.of("/proc/self/fdinfo/2");
    /** The access-mode bits of the open flags: O_RDONLY is 0, O_WRONLY 1 and O_RDWR 2. */
    private static final int ACCESS_MODE_MASK = 3;
    private static final int O_RDONLY = 0;
    private static final int O_INVALID_MODE = 3;
    private static final String FLAGS_KEY = "flags:";
    private static final int OCTAL = 8;

    private final Sink descriptor2;
    private final Sink second;
    private final BooleanSupplier descriptor2IsValid;

    private StderrSink(Sink descriptor2, Sink second, BooleanSupplier descriptor2IsValid) {
        this.descriptor2 = descriptor2;
        this.second = second;
        this.descriptor2IsValid = descriptor2IsValid;
    }

    /**
     * {@code allowRegularFile} is for tests: the real caller passes false. A path that is a regular file is used only when
     * {@code allowRegularFile} is set or {@code fdinfo} says descriptor 2 was opened for writing; a path that cannot be
     * opened is not used. With none left only descriptor 2 is used.
     */
    static Sink open(List<Path> candidates, OutputStream descriptor2, BooleanSupplier descriptor2IsValid,
            boolean allowRegularFile, Supplier<String> fdinfo) {
        Sink primary = new StreamSink(descriptor2);
        for (Path path : candidates) {
            try {
                if (!allowRegularFile && isRegularFile(path) && !openedForWriting(fdinfo.get())) {
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

    /** The same, for tests that do not look at fdinfo: a regular file is reopened only when {@code allowRegularFile}. */
    static Sink open(List<Path> candidates, OutputStream descriptor2, BooleanSupplier descriptor2IsValid, boolean allowRegularFile) {
        return open(candidates, descriptor2, descriptor2IsValid, allowRegularFile, () -> null);
    }

    private static boolean isRegularFile(Path path) throws IOException {
        return Files.readAttributes(path, BasicFileAttributes.class).isRegularFile();
    }

    /** The text of {@code fdinfo}, or null when it cannot be read (no {@code /proc}, or a filtered one). */
    static String readFdinfo(Path fdinfo) {
        try {
            return Files.readString(fdinfo, StandardCharsets.US_ASCII);
        } catch (IOException | RuntimeException unreadable) {
            return null;
        }
    }

    /** True only when the fdinfo text has a {@code flags:} line whose octal value has a write or read-write access mode. */
    static boolean openedForWriting(String fdinfo) {
        if (fdinfo == null) {
            return false;
        }
        for (String line : fdinfo.split("\n")) {
            if (line.startsWith(FLAGS_KEY)) {
                return flagsAllowWriting(line.substring(FLAGS_KEY.length()).trim());
            }
        }
        return false;
    }

    private static boolean flagsAllowWriting(String octal) {
        try {
            int mode = Integer.parseInt(octal, OCTAL) & ACCESS_MODE_MASK;
            return mode != O_RDONLY && mode != O_INVALID_MODE;
        } catch (NumberFormatException malformed) {
            return false;
        }
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
