package hairspring;

import java.util.Set;

/**
 * The source location of a call site: the first stack frame outside the SDK. Looked at once per site (the first time an
 * id is seen), never on the hot path.
 */
final class Where {
    static final String UNKNOWN_FILE = "<unknown>";
    private static final Set<String> INTERNAL = Set.of(Hairspring.class.getName(), Recorder.class.getName(), Where.class.getName());

    private Where() {}

    record Location(String file, long line) {}

    static Location caller() {
        return StackWalker.getInstance()
                .walk(frames -> frames.filter(f -> !INTERNAL.contains(f.getClassName())).findFirst())
                .map(frame -> {
                    String name = frame.getFileName();
                    if (name == null) {
                        return new Location(UNKNOWN_FILE, 0);
                    }
                    String cls = frame.getClassName();
                    int dot = cls.lastIndexOf('.');
                    String path = dot < 0 ? name : cls.substring(0, dot).replace('.', '/') + "/" + name;
                    return new Location(path, Math.max(frame.getLineNumber(), 0));
                })
                .orElse(new Location(UNKNOWN_FILE, 0));
    }
}
