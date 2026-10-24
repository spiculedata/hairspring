package langat;

import hairspring.Hairspring;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * The planted race: four platform threads behind a start barrier, each doing {@code iterations} times a load of one
 * shared counter, a busy loop and a store of the value read plus one.
 *
 * <p>The window is the three steps in {@link #racyIterations} (or {@link #lockedIterations}, which holds a monitor across
 * them): {@code long v = counter; long acc = spin(window); counter = v + 1;}. {@code counter} is a plain field, so the
 * load and the store are two separate operations and an update is lost whenever another worker runs between them. There
 * is no system call, allocation, lock, sleep, yield, clock read or SDK call between them; the SDK's work counter and the
 * busy loop's result are written after the store.
 *
 * <p>Why the window survives the JIT. Nothing in the language stops a compiler from moving the plain load past the busy
 * loop or keeping {@code counter} in a register, which would remove the window. Two things keep it here: the worker
 * loops run 1,000 iterations, too few for HotSpot to compile them (a compiled loop is what could do that); and the busy
 * loop's result goes to a volatile field after the store, so the loop itself cannot be dropped. The acceptance profiles
 * (an interpreter-only arm and a C1-only arm) do no such motion; natively the default JIT does not reach these loops
 * either. The build record's native runs are the evidence for the arm that ran, and the contract does not allow a
 * volatile or atomic counter instead.
 */
final class Race {
    private static final long SPIN_MULTIPLIER = 0x5851f42d4c957f2dL;
    /** A fixed non-zero start, so the accumulator never sticks at zero. */
    private static final long SPIN_SEED = 0xcbf29ce484222325L;
    private static final int SPIN_ROTATE = 7;

    /** Where the busy loop's result goes, so the loop has an effect. Written after the store, outside the window. */
    private static volatile long sink;

    private final Mode mode;
    private final int iterations;
    private final Object lock = new Object();
    private final AtomicInteger started = new AtomicInteger();
    private final AtomicInteger arrivals = new AtomicInteger();
    private final int[] arrival = new int[Contract.THREADS];
    private final StartBarrier barrier = new StartBarrier(started, Contract.THREADS);
    private volatile String failure;
    /** The shared counter: a plain, non-volatile field. */
    private long counter;

    private Race(Mode mode, int iterations) {
        this.mode = mode;
        this.iterations = iterations;
    }

    /** A busy loop in this file's own code: {@code rounds} rounds on an accumulator the caller keeps. */
    static long spin(int rounds) {
        long acc = SPIN_SEED;
        for (int i = 0; i < rounds; i++) {
            acc = (Long.rotateLeft(acc, SPIN_ROTATE) ^ i) * SPIN_MULTIPLIER;
        }
        return acc;
    }

    /** Runs the four workers to completion. */
    static RaceResult run(Mode mode, int iterations) throws RaceFailure {
        return new Race(mode, iterations).execute();
    }

    private void racyIterations() {
        int window = mode.window();
        for (int i = 0; i < iterations; i++) {
            long v = counter;
            long acc = spin(window);
            counter = v + 1;
            sink = acc;
            Hairspring.work(1);
        }
    }

    private void lockedIterations() {
        int window = mode.window();
        for (int i = 0; i < iterations; i++) {
            long acc;
            synchronized (lock) {
                long v = counter;
                acc = spin(window);
                counter = v + 1;
            }
            sink = acc;
            Hairspring.work(1);
        }
    }

    private void work(int index) {
        try {
            Hairspring.thread("worker", index);
            started.incrementAndGet();
            if (!barrier.await()) {
                return;
            }
            if (mode.locked()) {
                lockedIterations();
            } else {
                racyIterations();
            }
            arrival[arrivals.getAndIncrement()] = index;
        } catch (RuntimeException | Error problem) {
            failure = "worker " + index + ": " + problem;
            barrier.abort();
        }
    }

    private RaceResult execute() throws RaceFailure {
        Thread[] threads = new Thread[Contract.THREADS];
        int spawned = 0;
        try {
            for (int index = 0; index < Contract.THREADS; index++) {
                int worker = index;
                threads[index] = new Thread(() -> work(worker), "worker-" + index);
                threads[index].start();
                spawned++;
            }
        } catch (RuntimeException | OutOfMemoryError refused) {
            failure = "a thread did not start: " + refused;
            barrier.abort();
        }
        boolean joined = joinAll(threads, spawned);
        if (failure != null) {
            throw new RaceFailure(failure);
        }
        if (!joined || arrivals.get() != Contract.THREADS) {
            throw new RaceFailure("a worker thread failed to start or died");
        }
        return new RaceResult(counter, started.get(), arrival.clone());
    }

    /** Joins every started worker, so no result is read while one still runs. */
    private static boolean joinAll(Thread[] threads, int spawned) {
        boolean joined = true;
        for (int i = 0; i < spawned; i++) {
            try {
                threads[i].join();
            } catch (InterruptedException interrupted) {
                Thread.currentThread().interrupt();
                joined = false;
            }
        }
        return joined;
    }
}
