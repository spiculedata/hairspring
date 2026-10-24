package langat;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * State shared by the four workers: a monitor that records who entered it first and guards a counter, a counter with no
 * guard at all (a plain field read, spun on and written back, so a thread switch in the window loses an update), and a
 * queue that records who finished first.
 */
final class Shared {
    private static long sink;

    private final Object lock = new Object();
    private final List<Integer> lockOrder = new ArrayList<>();
    private final BlockingQueue<Integer> finished = new LinkedBlockingQueue<>();
    private final AtomicInteger started = new AtomicInteger();
    private volatile boolean abort;
    private long locked;
    private long racy;

    /** One worker: wait for the others, take the monitor once, run the rounds, report the finish. */
    void work(int id) {
        try {
            started.incrementAndGet();
            waitForAll();
            if (abort) {
                return;
            }
            synchronized (lock) {
                lockOrder.add(id);
            }
            for (int round = 0; round < Workload.ROUNDS; round++) {
                long seen = racy;
                sink = Workload.spin(Workload.SPIN_ROUNDS);
                racy = seen + 1;
                synchronized (lock) {
                    locked++;
                }
            }
            finished.add(id);
        } catch (RuntimeException | Error problem) {
            abort = true;
        }
    }

    private void waitForAll() {
        while (started.get() < Workload.THREADS && !abort) {
            Thread.yield();
        }
    }

    /** Called when a worker could not be started, so the others stop waiting. */
    void abort() {
        abort = true;
    }

    boolean aborted() {
        return abort;
    }

    /** Read after every worker is joined. */
    List<Integer> lockOrder() {
        synchronized (lock) {
            return List.copyOf(lockOrder);
        }
    }

    /** The finishing order, as the queue holds it. Read after every worker is joined. */
    List<Integer> arrival() {
        return List.copyOf(finished);
    }

    long racyFinal() {
        return racy;
    }

    long lockedFinal() {
        synchronized (lock) {
            return locked;
        }
    }
}
