package hairspring;

import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

/**
 * The process-wide state: sites, thread slots, the lock that guards them, and the report.
 *
 * <p>Every change to a counter happens with one monitor held, so counts are exact under any number of threads and a
 * snapshot taken under the same monitor shows every count, witness and thread total at one moment. The critical
 * sections are a few field updates; the report's lines are written outside the monitor. The fast path of a call is one
 * concurrent-map read to find the site and one {@link ThreadLocal} read to find the thread's slot; nothing is allocated
 * on it except what the caller passes (the values array).
 */
final class Recorder {
    private final Identity identity;
    private final Sink sink;
    private final long ownerPid;
    private final Object lock = new Object();
    private final ThreadLocal<Slot> local = new ThreadLocal<>();
    private final SiteTable sites = new SiteTable();
    private final SlotTable slots = new SlotTable();
    private long dropped;
    private volatile long status;
    private boolean reported;

    Recorder(Identity identity, Sink sink, long dropped, long ownerPid) {
        this.identity = identity;
        this.sink = sink;
        this.dropped = dropped;
        this.ownerPid = ownerPid;
    }

    // ---- starting up -------------------------------------------------------------------------------------------

    /** The recorder for a process: the tag comes from the environment; {@code openingLosses} are counted as dropped. */
    static Recorder fromEnvironment(Map<String, String> env, Sink sink, long openingLosses, long pid) {
        Identity identity = Identity.resolve(env.get(Identity.ENV_TAG), env.get(Identity.ENV_PID), pid);
        return new Recorder(identity, sink, openingLosses + (identity.refused() ? 1 : 0), pid);
    }

    /** The recorder for this process: tag and output from the environment, the report registered as a shutdown hook. */
    static Recorder start() {
        Map<String, String> env = System.getenv();
        Output.Opened opened = Output.open(env, Secure.isPrivileged(), new FileOutputStream(FileDescriptor.err));
        Recorder recorder = fromEnvironment(env, opened.sink(), opened.dropped(), ProcessHandle.current().pid());
        recorder.installShutdownHook();
        return recorder;
    }

    void installShutdownHook() {
        Thread hook = new Thread(this::reportAtExit, "hairspring-report");
        try {
            Runtime.getRuntime().addShutdownHook(hook);
        } catch (IllegalStateException shuttingDown) {
            // The JVM is already exiting: there is nothing left to hook.
        }
    }

    private void reportAtExit() {
        try {
            report(EndReason.EXIT, null);
        } catch (RuntimeException | Error failed) {
            // Not a protocol line, so a parser ignores it; the missing end line is what the engine sees.
            System.err.println("hairspring: the report could not be written: " + failed);
        }
    }

    // ---- state -------------------------------------------------------------------------------------------------

    void setStatus(long value) {
        status = value;
    }

    private void noteDrop() {
        synchronized (lock) {
            dropped++;
        }
    }

    private Slot claimSlot() {
        Slot slot;
        synchronized (lock) {
            SlotTable.Claim claim = slots.claim();
            slot = claim.slot();
            if (claim.shared()) {
                dropped++; // a thread that shares the overflow slot
            }
        }
        local.set(slot);
        return slot;
    }

    private Slot slot() {
        Slot slot = local.get();
        return slot != null ? slot : claimSlot();
    }

    /** The site for an id and kind, registered (with the caller's location) when it is new; null when the table is full. */
    private Site site(Kind kind, String id, String msg) {
        Site site = sites.find(kind, id);
        if (site != null) {
            return site == Site.UNTRACKED ? null : site;
        }
        Where.Location where = Where.caller();
        synchronized (lock) {
            return sites.register(kind, id, msg, where.file(), where.line());
        }
    }

    // ---- the call sites ----------------------------------------------------------------------------------------

    /**
     * Put a site in the catalog without evaluating it, so a site that is never reached still shows up (and a
     * {@code reachable} that is never reached is reported as unmet).
     */
    void declare(Kind kind, String id, String msg) {
        if (site(kind, id, msg) == null) {
            noteDrop();
        }
    }

    /**
     * Count one evaluation. The first evaluation on each side of a site keeps its witness. A point kind
     * ({@code reachable}, {@code unreachable}) has no condition: reaching it counts as true.
     */
    void record(Kind kind, boolean ok, String id, String msg, long[] vals) {
        Args.checkVals(vals);
        Site site = site(kind, id, msg);
        Slot slot = slot();
        synchronized (lock) {
            if (site == null) {
                dropped++; // more distinct ids than the catalog holds
                return;
            }
            slot.evals++;
            if (ok || site.point) {
                site.trues++;
                if (site.firstTrue == null) {
                    site.firstTrue = new Site.Mark(slot.index, slot.evals, Args.packVals(vals));
                }
            } else {
                site.falses++;
                if (site.firstFalse == null) {
                    site.firstFalse = new Site.Mark(slot.index, slot.evals, Args.packVals(vals));
                }
            }
        }
    }

    /** Add {@code n} operations to this thread's work count. */
    void work(long n) {
        if (n < 0) {
            throw new IllegalArgumentException("work cannot be negative");
        }
        Slot slot = slot();
        synchronized (lock) {
            slot.work += n;
        }
    }

    /** Give the calling thread the stable key {@code label-index}. See {@link SlotTable#setKey}. */
    boolean thread(String label, long index) {
        Slot slot = slot();
        synchronized (lock) {
            return slots.setKey(slot, label, index);
        }
    }

    // ---- the report --------------------------------------------------------------------------------------------

    private Rows.Witness witness(Site.Mark mark) {
        return mark == null ? null : new Rows.Witness(slots.keyAt(mark.slot()), mark.ord(), mark.vals());
    }

    /** Everything the report says, read at one moment. */
    Snapshot snapshot() {
        synchronized (lock) {
            List<Site> all = sites.all();
            List<Rows.CatalogRow> catalog = new ArrayList<>(all.size());
            List<Catalog.Entry> entries = new ArrayList<>(all.size());
            for (Site s : all) {
                catalog.add(new Rows.CatalogRow(s.id, s.kind, s.msg, s.file, s.line));
                entries.add(new Catalog.Entry(s.id, s.kind));
            }
            List<Rows.AssertRow> asserts = new ArrayList<>();
            for (Site s : sites.canonical()) {
                asserts.add(new Rows.AssertRow(s.id, s.kind, s.trues, s.falses, witness(s.firstTrue), witness(s.firstFalse)));
            }
            return new Snapshot(
                    List.copyOf(catalog),
                    List.copyOf(asserts),
                    slots.views(),
                    Catalog.hash(entries),
                    sites.distinctIds(),
                    dropped);
        }
    }

    /**
     * Write the report: at most once per process, and only from the process that started the SDK. Returns whether this
     * call wrote it. {@code statusOverride} replaces the status the program recorded.
     */
    boolean report(EndReason reason, Long statusOverride) {
        synchronized (lock) {
            if (reported) {
                return false;
            }
            reported = true;
        }
        if (ProcessHandle.current().pid() != ownerPid || sink == null) {
            return false;
        }
        Report.write(snapshot(), identity, sink, reason, statusOverride == null ? status : statusOverride, this::noteDrop);
        return true;
    }

    /** The environment a child process needs to take its tag from this one (Java cannot change its own environment). */
    Map<String, String> childEnvironment() {
        boolean announced;
        synchronized (lock) {
            announced = reported;
        }
        return Map.of(
                Identity.ENV_TAG, identity.tag(),
                Identity.ENV_PID, Identity.ownerField(ownerPid, announced, identity.tagStable()));
    }
}
