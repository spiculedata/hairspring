package langat;

import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.concurrent.ConcurrentHashMap;

/** The workload: four platform threads on a {@link Shared}, a map, a random value and the elapsed monotonic time. */
final class Workload {
    static final int THREADS = 4;
    static final int ROUNDS = 200;
    static final int SPIN_ROUNDS = 1_000;
    static final int MAP_ENTRIES = 32;

    private static final long SPIN_MULTIPLIER = 0x5851f42d4c957f2dL;
    /** A fixed non-zero start, so the accumulator never sticks at zero. */
    private static final long SPIN_SEED = 0xcbf29ce484222325L;
    private static final int SPIN_ROTATE = 7;

    private Workload() {}

    /** A busy loop of user code with an accumulator that depends on every round. */
    static long spin(int rounds) {
        long acc = SPIN_SEED;
        for (int i = 0; i < rounds; i++) {
            acc = (Long.rotateLeft(acc, SPIN_ROTATE) ^ i) * SPIN_MULTIPLIER;
        }
        return acc;
    }

    /**
     * The keys of a {@link ConcurrentHashMap} of {@code MAP_ENTRIES} entries in iteration order: a function of the
     * keys' hash codes and the table alone, so it is the same every time on one JDK.
     */
    static List<String> mapOrder() {
        Map<String, Integer> map = new ConcurrentHashMap<>();
        for (int key = 0; key < MAP_ENTRIES; key++) {
            map.put("key-" + key, key);
        }
        return List.copyOf(map.keySet());
    }

    /** Run the four workers to completion and collect what the line reports. */
    static Outcome run() throws InvariantException {
        Shared shared = new Shared();
        Thread[] threads = new Thread[THREADS];
        long startedAt = System.nanoTime();
        int spawned = startAll(shared, threads);
        boolean joined = joinAll(threads, spawned);
        long endedAt = System.nanoTime();
        if (spawned != THREADS || !joined || shared.aborted()) {
            throw new InvariantException("a worker thread failed to start or died");
        }
        List<Integer> lockOrder = shared.lockOrder();
        List<Integer> arrival = shared.arrival();
        if (lockOrder.size() != THREADS || arrival.size() != THREADS) {
            throw new InvariantException("a worker did not take the lock and finish");
        }
        return new Outcome(
                lockOrder,
                arrival,
                shared.racyFinal(),
                shared.lockedFinal(),
                endedAt - startedAt,
                mapOrder(),
                new Random().nextLong());
    }

    /** Start the workers; returns how many started (all of them unless the runtime refused a thread). */
    private static int startAll(Shared shared, Thread[] threads) {
        int spawned = 0;
        try {
            for (int id = 0; id < THREADS; id++) {
                int worker = id;
                threads[id] = new Thread(() -> shared.work(worker), "probe-" + id);
                threads[id].start();
                spawned++;
            }
        } catch (RuntimeException | OutOfMemoryError refused) {
            shared.abort();
        }
        return spawned;
    }

    /** Join every started worker, so no result is read while one still runs. */
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
