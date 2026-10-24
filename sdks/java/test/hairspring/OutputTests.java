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
