package hairspring;

import java.io.IOException;
import java.io.OutputStream;
import java.nio.file.Path;
import java.util.Map;

/**
 * Chooses where the report goes, from the environment: {@code HAIRSPRING_EVENTS=off} writes nothing, a usable
 * {@code HAIRSPRING_EVENTS_FILE} is appended to, anything else goes to standard error. A process started with elevated
 * rights ignores the file setting, because the SDK would open a path from the environment for writing.
 */
final class Output {
    static final String ENV_EVENTS = "HAIRSPRING_EVENTS";
    static final String ENV_EVENTS_FILE = "HAIRSPRING_EVENTS_FILE";
    private static final String OFF = "off";

    private Output() {}

    /** The sink (null when output is off) and how many losses to count as dropped (an events file that could not be opened). */
    record Opened(Sink sink, long dropped) {}

    static Opened open(Map<String, String> environ, boolean privileged, OutputStream stderr) {
        return open(environ, privileged, new StreamSink(stderr));
    }

    static Opened open(Map<String, String> environ, boolean privileged, Sink stderr) {
        if (OFF.equals(environ.get(ENV_EVENTS))) {
            return new Opened(null, 0);
        }
        long dropped = 0;
        String path = privileged ? null : environ.get(ENV_EVENTS_FILE);
        if (path != null && !path.isEmpty()) {
            try {
                return new Opened(EventsFile.open(Path.of(path)), 0);
            } catch (IOException | RuntimeException refused) {
                dropped = 1; // the file could not be used: events fall back to standard error, and this is counted
            }
        }
        return new Opened(stderr, dropped);
    }
}
