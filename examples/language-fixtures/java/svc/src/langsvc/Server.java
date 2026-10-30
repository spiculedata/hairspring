package langsvc;

import hairspring.Hairspring;
import java.util.concurrent.Executors;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ThreadFactory;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;

/**
 * The service: a fixed pool of worker threads that take delivered copies from an unbounded queue, drop expired ones,
 * decode the payload and hand it to the store; and a ticker thread that samples the queue depth every few milliseconds
 * (a metrics timer, so a timer thread runs beside the workers; it changes nothing the property reads).
 */
final class Server {
    private final Store store;
    private final Mode mode;
    private final ThreadPoolExecutor pool;
    private final ScheduledExecutorService ticker;
    private final AtomicLong expired = new AtomicLong();
    private final AtomicLong ticks = new AtomicLong();
    private volatile long maxDepth;
    private volatile String failure;

    Server(Store store, Mode mode) {
        this.store = store;
        this.mode = mode;
        this.pool = new ThreadPoolExecutor(Contract.WORKERS, Contract.WORKERS, 0L, TimeUnit.MILLISECONDS,
                new LinkedBlockingQueue<>(), named("worker"));
        this.ticker = Executors.newSingleThreadScheduledExecutor(named("ticker"));
    }

    /** A thread factory that gives each thread a Hairspring key before it runs anything. */
    private static ThreadFactory named(String label) {
        AtomicInteger next = new AtomicInteger();
        return body -> {
            int index = next.getAndIncrement();
            Thread thread = new Thread(() -> {
                Hairspring.thread(label, index);
                body.run();
            }, label + "-" + index);
            thread.setDaemon(false);
            return thread;
        };
    }

    void start() {
        pool.prestartAllCoreThreads();
        ticker.scheduleAtFixedRate(this::tick, Contract.TICK_MS, Contract.TICK_MS, TimeUnit.MILLISECONDS);
    }

    /** Delivers one copy. */
    void deliver(Messages.Request request) {
        pool.execute(() -> handle(request));
    }

    /** Handles one copy; a handler that throws is recorded and answered FAILED, so no client waits for a reply that never comes. */
    private void handle(Messages.Request request) {
        long key = -1;
        Messages.Status status;
        try {
            Messages.Transfer t = Messages.decode(request.payload());
            key = t.key();
            if (Deadline.passed(request.deadline())) {
                expired.incrementAndGet();
                status = Messages.Status.EXPIRED;
            } else {
                status = store.handle(t, mode);
            }
        } catch (RuntimeException | Error problem) {
            failure = "handler: " + problem;
            status = Messages.Status.FAILED;
        }
        request.replyTo().add(new Messages.Reply(key, status));
    }

    private void tick() {
        ticks.incrementAndGet();
        long depth = pool.getQueue().size();
        if (depth > maxDepth) {
            maxDepth = depth;
        }
    }

    /** Stops taking work, lets the queued copies finish, stops the ticker. False if either would not stop in time. */
    boolean drain() throws InterruptedException {
        pool.shutdown();
        ticker.shutdown();
        for (int step = 0; step < Contract.DRAIN_STEPS; step++) {
            boolean poolDone = pool.awaitTermination(Contract.DRAIN_STEP_MS, TimeUnit.MILLISECONDS);
            boolean tickerDone = ticker.awaitTermination(Contract.DRAIN_STEP_MS, TimeUnit.MILLISECONDS);
            if (poolDone && tickerDone) {
                return true;
            }
        }
        return false;
    }

    long expired() {
        return expired.get();
    }

    long ticks() {
        return ticks.get();
    }

    long maxDepth() {
        return maxDepth;
    }

    String failure() {
        return failure;
    }
}
