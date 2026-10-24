package hairspring;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.function.Predicate;

/** A sink that keeps the lines it is given, for tests. {@code refuse} makes it fail for the lines it matches. */
final class ListSink implements Sink {
    private final List<String> lines = Collections.synchronizedList(new ArrayList<>());
    private final Predicate<String> refuse;

    ListSink() {
        this(line -> false);
    }

    ListSink(Predicate<String> refuse) {
        this.refuse = refuse;
    }

    @Override
    public boolean writeLine(byte[] line) {
        String text = new String(line, StandardCharsets.US_ASCII);
        if (refuse.test(text)) {
            return false;
        }
        lines.add(text);
        return true;
    }

    @Override
    public void close() {}

    List<String> lines() {
        synchronized (lines) {
            return List.copyOf(lines);
        }
    }

    /** The lines of one kind, in the order written. */
    List<String> ofKind(String kind) {
        String needle = "\"kind\":\"" + kind + "\"";
        return lines().stream().filter(l -> l.contains(needle)).toList();
    }

    /** The kinds in the order written, one entry per line. */
    List<String> kinds() {
        return lines().stream().map(l -> l.substring(l.indexOf("\"kind\":\"") + 8, l.indexOf('"', l.indexOf("\"kind\":\"") + 8))).toList();
    }
}
