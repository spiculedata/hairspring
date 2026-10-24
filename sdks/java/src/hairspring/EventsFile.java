package hairspring;

import java.io.IOException;
import java.nio.channels.FileChannel;
import java.nio.file.LinkOption;
import java.nio.file.NoSuchFileException;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.nio.file.attribute.BasicFileAttributes;
import java.nio.file.attribute.PosixFilePermissions;
import java.util.Objects;
import java.util.Set;

/**
 * Opens {@code HAIRSPRING_EVENTS_FILE}. The file must be a regular file and is never reached through a link in its last
 * component: it is looked at without following links first (a FIFO, a device or a directory is refused before anything
 * is opened, so a FIFO with no reader cannot hang the program), opened for appending with {@code O_NOFOLLOW}, and looked
 * at again (a different file, or one that stopped being regular, is refused).
 *
 * <p>Java has no {@code O_NONBLOCK} for files, so the look before the open is what keeps a FIFO from blocking; a FIFO
 * swapped in between the two steps is not caught. Run the program under test in a directory no one else can write to.
 */
final class EventsFile {
    private static final String MODE = "rw-r--r--";

    private EventsFile() {}

    private static BasicFileAttributes lookWithoutFollowing(Path path) throws IOException {
        return java.nio.file.Files.readAttributes(path, BasicFileAttributes.class, LinkOption.NOFOLLOW_LINKS);
    }

    static Sink open(Path path) throws IOException {
        BasicFileAttributes before = null;
        try {
            before = lookWithoutFollowing(path);
        } catch (NoSuchFileException missing) {
            // The file is created below.
        }
        if (before != null && !before.isRegularFile()) {
            throw new IOException("the events file is not a regular file: " + path);
        }
        FileChannel channel = FileChannel.open(
                path,
                Set.of(StandardOpenOption.WRITE, StandardOpenOption.APPEND, StandardOpenOption.CREATE, LinkOption.NOFOLLOW_LINKS),
                PosixFilePermissions.asFileAttribute(PosixFilePermissions.fromString(MODE)));
        try {
            BasicFileAttributes after = lookWithoutFollowing(path);
            if (!after.isRegularFile()) {
                throw new IOException("the events file is not a regular file: " + path);
            }
            if (before != null && !Objects.equals(before.fileKey(), after.fileKey())) {
                throw new IOException("the events file changed while it was being opened: " + path);
            }
        } catch (IOException | RuntimeException refused) {
            channel.close();
            throw refused;
        }
        return new ChannelSink(channel);
    }
}
