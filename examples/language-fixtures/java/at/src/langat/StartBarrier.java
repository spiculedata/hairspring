package langat;

import java.util.concurrent.atomic.AtomicInteger;

/**
 * The start barrier: each worker first counts itself in {@code started}, then waits here until every party has. It yields
 * the processor while it waits (a system call, which the contract allows before the first iteration) and has no timeout:
 * a worker that fails, or a thread that cannot be started, aborts it, which wakes the others and tells them the start
 * failed.
 */
final class StartBarrier {
    private final AtomicInteger started;
    private final int parties;
    private volatile boolean aborted;

    StartBarrier(AtomicInteger started, int parties) {
        this.started = started;
        this.parties = parties;
    }

    /** Waits until {@code started} reaches the number of parties. True when it did, false when the barrier was aborted. */
    boolean await() {
        while (started.get() < parties && !aborted) {
            Thread.yield();
        }
        return !aborted;
    }

    void abort() {
        aborted = true;
    }
}
