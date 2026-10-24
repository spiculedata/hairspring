package hairspring;

import java.util.function.Supplier;

/** The lines written from a {@link Snapshot}, without any lock held. */
final class Report {
    private Report() {}

    /** Sends lines and counts the ones that were lost. A line that does not fit, or cannot be written, is counted, never truncated. */
    private static final class Writer {
        private final Sink sink;
        private final Runnable noteDrop;
        private int lost;

        Writer(Sink sink, Runnable noteDrop) {
            this.sink = sink;
            this.noteDrop = noteDrop;
        }

        void send(Supplier<byte[]> build) {
            boolean written;
            try {
                written = sink.writeLine(build.get());
            } catch (LineTooLongException tooLong) {
                written = false;
            }
            if (!written) {
                lost++;
                noteDrop.run();
            }
        }

        int lost() {
            return lost;
        }
    }

    /**
     * Writes hello, one catalog line per (id, kind), one assert per id, work, a thread line per thread, then end.
     * {@code snapshot.dropped()} is the loss count before the report; {@code end} adds the lines lost during it.
     * {@code noteDrop} is called for every lost line.
     */
    static void write(Snapshot snapshot, Identity identity, Sink sink, EndReason reason, long status, Runnable noteDrop) {
        Rows.Proc info = new Rows.Proc(
                identity.tag(), identity.tagStable(), false, snapshot.catalogHash(), snapshot.nIds(), snapshot.dropped());
        Writer writer = new Writer(sink, noteDrop);
        writer.send(() -> Lines.hello(info));
        for (Rows.CatalogRow row : snapshot.catalog()) {
            writer.send(() -> Lines.catalog(info, row));
        }
        for (Rows.AssertRow row : snapshot.asserts()) {
            writer.send(() -> Lines.assertLine(info, row));
        }
        writer.send(() -> Lines.work(info, snapshot.work()));
        for (ThreadView view : snapshot.threads()) {
            writer.send(() -> Lines.thread(info, new Rows.ThreadRow(view.key(), view.evals(), view.tapeConsumed(), view.stable())));
        }
        Rows.Proc closing = new Rows.Proc(
                info.tag(), info.tagStable(), info.signalHandlers(), info.catalogHash(), info.nIds(), info.dropped() + writer.lost());
        writer.send(() -> Lines.end(closing, reason, status, snapshot.evals(), snapshot.threads().size()));
    }
}
