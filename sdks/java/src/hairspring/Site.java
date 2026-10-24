package hairspring;

/**
 * One (id, kind) pair and its counters. {@code id} is the form that is sent. The identity fields never change; the
 * counters and witnesses are changed only with the recorder's lock held.
 */
final class Site {
    /** Stands in the lookup for an id the table had no room for, so the hot path recognises it without registering again. */
    static final Site UNTRACKED = new Site("", Kind.REACHABLE, "", "", 0);

    /** The first evaluation on one side of a site: which slot, that thread's ordinal and the four values. */
    record Mark(int slot, long ord, long[] vals) {}

    final String id;
    final Kind kind;
    final boolean point;
    final String msg;
    final String file;
    final long line;
    long trues;
    long falses;
    Mark firstTrue;
    Mark firstFalse;

    Site(String id, Kind kind, String msg, String file, long line) {
        this.id = id;
        this.kind = kind;
        this.point = kind.isPoint();
        this.msg = msg;
        this.file = file;
        this.line = line;
    }
}
