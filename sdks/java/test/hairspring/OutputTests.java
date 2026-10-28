package hairspring;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.attribute.PosixFilePermission;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;

/** Where the lines go: standard error, or an events file that must be a regular file and is never followed through a link. */
final class OutputTests {
    private static final long BLOCKED_SECONDS = 10;

    private OutputTests() {}

    /** What Linux shows in /proc/self/fdinfo/2 for a descriptor with these open flags (octal). */
    private static String fdinfo(String octalFlags) {
        return "pos:\t0\nflags:\t" + octalFlags + "\nmnt_id:\t30\nino:\t1234\n";
    }

    private static byte[] bytes(String text) {
        return text.getBytes(StandardCharsets.US_ASCII);
    }

    private static Path scratch() throws IOException {
        return Files.createTempDirectory("hs-java-output-");
    }

    private static void remove(Path dir) throws IOException {
        try (var walk = Files.walk(dir)) {
            for (Path p : walk.sorted(java.util.Comparator.reverseOrder()).toList()) {
                Files.deleteIfExists(p);
            }
        }
    }

    /** Opens the events file on another thread: a FIFO opened for writing with no reader would block for ever. */
    private static Sink openWithin(Path file) throws Exception {
        ExecutorService pool = Executors.newSingleThreadExecutor(r -> {
            Thread t = new Thread(r, "open-events-file");
            t.setDaemon(true);
            return t;
        });
        try {
            Future<Sink> opening = pool.submit(() -> EventsFile.open(file));
            try {
                return opening.get(BLOCKED_SECONDS, TimeUnit.SECONDS);
            } catch (java.util.concurrent.ExecutionException failed) {
                if (failed.getCause() instanceof IOException io) {
                    throw io;
                }
                throw failed;
            }
        } finally {
            pool.shutdownNow();
        }
    }

    static void register(Suite s) {
        s.add("output: a stream sink writes the whole line and says so", () -> {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            Sink sink = new StreamSink(out);
            Check.yes(sink.writeLine(bytes("one\n")), "written");
            Check.equal("one\n", out.toString(StandardCharsets.US_ASCII));
        });
        s.add("output: a stream that fails costs the line, not the process", () -> {
            Sink sink = new StreamSink(new OutputStream() {
                @Override
                public void write(int b) throws IOException {
                    throw new IOException("Broken pipe");
                }
            });
            Check.no(sink.writeLine(bytes("one\n")), "written");
        });
        s.add("output: the events file receives appended lines in order", () -> {
            Path dir = scratch();
            try {
                Path file = dir.resolve("events");
                Files.writeString(file, "before\n");
                Sink sink = openWithin(file);
                Check.yes(sink.writeLine(bytes("a\n")), "a");
                Check.yes(sink.writeLine(bytes("b\n")), "b");
                sink.close();
                Check.equal("before\na\nb\n", Files.readString(file));
            } finally {
                remove(dir);
            }
        });
        s.add("output: a new events file is created readable by others but not writable", () -> {
            Path dir = scratch();
            try {
                Path file = dir.resolve("events");
                openWithin(file).close();
                Set<PosixFilePermission> mode = Files.getPosixFilePermissions(file);
                Check.yes(mode.contains(PosixFilePermission.OWNER_WRITE), "owner writes");
                Check.no(mode.contains(PosixFilePermission.GROUP_WRITE) && mode.contains(PosixFilePermission.OTHERS_WRITE), "everyone writes");
            } finally {
                remove(dir);
            }
        });
        s.add("output: a symbolic link as the events file is refused", () -> {
            Path dir = scratch();
            try {
                Path target = dir.resolve("target");
                Files.writeString(target, "x\n");
                Path link = dir.resolve("link");
                Files.createSymbolicLink(link, target);
                Check.raises(IOException.class, () -> openWithin(link));
                Check.equal("x\n", Files.readString(target));
            } finally {
                remove(dir);
            }
        });
        s.add("output: a directory as the events file is refused", () -> {
            Path dir = scratch();
            try {
                Check.raises(IOException.class, () -> openWithin(dir));
            } finally {
                remove(dir);
            }
        });
        s.add("output: a FIFO with no reader is refused without blocking", () -> {
            Path dir = scratch();
            try {
                Path fifo = dir.resolve("fifo");
                Process made = new ProcessBuilder("mkfifo", fifo.toString()).start();
                if (made.waitFor() != 0) {
                    System.out.println("     (skipped: no mkfifo here)");
                    return;
                }
                Check.raises(IOException.class, () -> openWithin(fifo));
            } finally {
                remove(dir);
            }
        });
        s.add("output: a device as the events file is refused", () -> {
            Path devnull = Path.of("/dev/null");
            if (!Files.exists(devnull)) {
                System.out.println("     (skipped: no /dev/null here)");
                return;
            }
            Check.raises(IOException.class, () -> openWithin(devnull));
        });
        s.add("output: while descriptor 2 is valid the line goes through it, not through the second channel", () -> {
            Path dir = scratch();
            try {
                Path file = dir.resolve("err");
                Files.writeString(file, "");
                ByteArrayOutputStream descriptor2 = new ByteArrayOutputStream();
                Sink sink = StderrSink.open(java.util.List.of(file), descriptor2, () -> true, true);
                Check.yes(sink.writeLine(bytes("a\n")), "a");
                Check.equal("a\n", descriptor2.toString(StandardCharsets.US_ASCII));
                Check.equal("", Files.readString(file));
            } finally {
                remove(dir);
            }
        });
        s.add("output: once descriptor 2 is closed the line goes to the second channel, appended", () -> {
            Path dir = scratch();
            try {
                Path file = dir.resolve("err");
                Files.writeString(file, "before\n");
                ByteArrayOutputStream descriptor2 = new ByteArrayOutputStream();
                boolean[] valid = {true};
                Sink sink = StderrSink.open(java.util.List.of(dir.resolve("missing"), file), descriptor2, () -> valid[0], true);
                valid[0] = false;
                Check.yes(sink.writeLine(bytes("a\n")), "a");
                Check.yes(sink.writeLine(bytes("b\n")), "b");
                sink.close();
                Check.equal("before\na\nb\n", Files.readString(file));
                Check.equal("", descriptor2.toString(StandardCharsets.US_ASCII));
            } finally {
                remove(dir);
            }
        });
        s.add("output: a regular file behind descriptor 2 is never reopened (it may be a file the JVM has open read-only)", () -> {
            Path dir = scratch();
            try {
                Path file = dir.resolve("err");
                Files.writeString(file, "keep\n");
                ByteArrayOutputStream descriptor2 = new ByteArrayOutputStream();
                Sink sink = StderrSink.open(java.util.List.of(file), descriptor2, () -> false, false);
                sink.writeLine(bytes("a\n"));
                sink.close();
                Check.equal("keep\n", Files.readString(file));
            } finally {
                remove(dir);
            }
        });
        s.add("output: fdinfo text of a descriptor opened for writing is read as writable, read-only is not", () -> {
            Check.yes(StderrSink.openedForWriting(fdinfo("0100001")), "O_WRONLY");
            Check.yes(StderrSink.openedForWriting(fdinfo("0102002")), "O_RDWR with other flags");
            Check.yes(StderrSink.openedForWriting(fdinfo("02101")), "O_WRONLY | O_CREAT | O_TRUNC");
            Check.yes(!StderrSink.openedForWriting(fdinfo("0100000")), "O_RDONLY");
            Check.yes(!StderrSink.openedForWriting(fdinfo("0100003")), "access mode 3 is not a mode");
            Check.yes(!StderrSink.openedForWriting(null), "no fdinfo");
            Check.yes(!StderrSink.openedForWriting(""), "empty fdinfo");
            Check.yes(!StderrSink.openedForWriting("pos:\t0\nmnt_id:\t30\n"), "no flags line");
            Check.yes(!StderrSink.openedForWriting("flags:\tnot-octal\n"), "flags that are not octal");
            Check.yes(!StderrSink.openedForWriting("flags:\t\n"), "flags with no value");
        });
        s.add("output: a regular file that fdinfo says was opened for writing is reopened, and the line goes there after the close", () -> {
            Path dir = scratch();
            try {
                Path file = dir.resolve("err");
                Files.writeString(file, "before\n");
                ByteArrayOutputStream descriptor2 = new ByteArrayOutputStream();
                boolean[] valid = {true};
                Sink sink = StderrSink.open(java.util.List.of(file), descriptor2, () -> valid[0], false, () -> fdinfo("0100001"));
                Check.yes(sink.writeLine(bytes("a\n")), "a");
                valid[0] = false;
                Check.yes(sink.writeLine(bytes("b\n")), "b");
                sink.close();
                Check.equal("a\n", descriptor2.toString(StandardCharsets.US_ASCII));
                Check.equal("before\nb\n", Files.readString(file));
            } finally {
                remove(dir);
            }
        });
        s.add("output: a regular file that fdinfo says was opened read-only is never written, whatever the file's mode", () -> {
            Path dir = scratch();
            try {
                Path file = dir.resolve("err");
                Files.writeString(file, "keep\n");
                ByteArrayOutputStream descriptor2 = new ByteArrayOutputStream();
                Sink sink = StderrSink.open(java.util.List.of(file), descriptor2, () -> false, false, () -> fdinfo("0100000"));
                sink.writeLine(bytes("a\n"));
                sink.close();
                Check.equal("keep\n", Files.readString(file));
            } finally {
                remove(dir);
            }
        });
        s.add("output: a regular file is never written when fdinfo is unavailable (no /proc, as on macOS)", () -> {
            Path dir = scratch();
            try {
                Path file = dir.resolve("err");
                Files.writeString(file, "keep\n");
                ByteArrayOutputStream descriptor2 = new ByteArrayOutputStream();
                Sink sink = StderrSink.open(java.util.List.of(file), descriptor2, () -> false, false, () -> null);
                sink.writeLine(bytes("a\n"));
                sink.close();
                Check.equal("keep\n", Files.readString(file));
            } finally {
                remove(dir);
            }
        });
        s.add("output: a reader of /proc/self/fdinfo/2 that is missing gives null, not an error", () -> {
            Check.equal(null, StderrSink.readFdinfo(Path.of("/nonexistent-dir-for-hairspring/fdinfo/2")));
        });
        s.add("output: the fdinfo file of descriptor 2 is the process's own", () -> {
            Check.equal(Path.of("/proc/self/fdinfo/2"), StderrSink.OWN_FDINFO);
        });
        s.add("output: without a usable path to descriptor 2 the stream on it is used", () -> {
            ByteArrayOutputStream fallback = new ByteArrayOutputStream();
            Sink sink = StderrSink.open(java.util.List.of(Path.of("/nonexistent-dir-for-hairspring/fd2")), fallback, () -> false, true);
            Check.yes(sink.writeLine(bytes("x\n")), "written");
            Check.equal("x\n", fallback.toString(StandardCharsets.US_ASCII));
        });
        s.add("output: the paths tried for descriptor 2 are the process's own", () -> {
            Check.equal(java.util.List.of(Path.of("/proc/self/fd/2"), Path.of("/dev/fd/2")), StderrSink.OWN_PATHS);
        });
        s.add("output: a missing directory is an error the caller counts", () ->
                Check.raises(IOException.class, () -> openWithin(Path.of("/nonexistent-dir-for-hairspring/events"))));
        s.add("output: events off writes nothing and costs nothing", () -> {
            Output.Opened opened = Output.open(Map.of("HAIRSPRING_EVENTS", "off"), false, new ByteArrayOutputStream());
            Check.yes(opened.sink() == null, "no sink");
            Check.equal(0L, opened.dropped());
        });
        s.add("output: without a file the report goes to standard error", () -> {
            ByteArrayOutputStream err = new ByteArrayOutputStream();
            Output.Opened opened = Output.open(Map.of(), false, err);
            opened.sink().writeLine(bytes("x\n"));
            Check.equal("x\n", err.toString(StandardCharsets.US_ASCII));
            Check.equal(0L, opened.dropped());
        });
        s.add("output: an events file that cannot be opened falls back to standard error and is counted", () -> {
            ByteArrayOutputStream err = new ByteArrayOutputStream();
            Output.Opened opened = Output.open(Map.of("HAIRSPRING_EVENTS_FILE", "/nonexistent-dir-for-hairspring/events"), false, err);
            opened.sink().writeLine(bytes("x\n"));
            Check.equal("x\n", err.toString(StandardCharsets.US_ASCII));
            Check.equal(1L, opened.dropped());
        });
        s.add("output: a privileged process ignores the events file setting", () -> {
            Path dir = scratch();
            try {
                Path file = dir.resolve("events");
                ByteArrayOutputStream err = new ByteArrayOutputStream();
                Output.Opened opened = Output.open(Map.of("HAIRSPRING_EVENTS_FILE", file.toString()), true, err);
                opened.sink().writeLine(bytes("x\n"));
                Check.equal("x\n", err.toString(StandardCharsets.US_ASCII));
                Check.no(Files.exists(file), "the file was created");
                Check.equal(0L, opened.dropped());
            } finally {
                remove(dir);
            }
        });
        s.add("output: the events file is used when it can be opened", () -> {
            Path dir = scratch();
            try {
                Path file = dir.resolve("events");
                ByteArrayOutputStream err = new ByteArrayOutputStream();
                Output.Opened opened = Output.open(Map.of("HAIRSPRING_EVENTS_FILE", file.toString()), false, err);
                opened.sink().writeLine(bytes("x\n"));
                opened.sink().close();
                Check.equal("x\n", Files.readString(file));
                Check.equal("", err.toString(StandardCharsets.US_ASCII));
                Check.equal(0L, opened.dropped());
            } finally {
                remove(dir);
            }
        });
    }
}
