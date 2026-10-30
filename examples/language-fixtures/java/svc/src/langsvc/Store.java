package langsvc;

import hairspring.Hairspring;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.locks.ReentrantLock;

/**
 * The service's small database: account balances and an applied-transfer count behind one lock (correct), and an
 * idempotency table that says which transfer keys have been applied.
 *
 * <p>The seeded bug is the claim on the idempotency table. The buggy build checks {@code containsKey} and records the key
 * only after applying the transfer, so two copies of one transfer handled at once by two workers can both pass the check
 * and both apply: a double charge. The fixed build claims the key first with one atomic {@code putIfAbsent}. Between the
 * check and the record the buggy build does its "validation" (a busy loop of {@code window} rounds) and the apply.
 */
final class Store {
    private static final long SPIN_MULTIPLIER = 0x5851f42d4c957f2dL;
    private static final long SPIN_SEED = 0xcbf29ce484222325L;
    private static final Long CLAIMED = -1L;
    private static volatile long sink;

    private final ConcurrentHashMap<Long, Long> applied = new ConcurrentHashMap<>();
    private final ReentrantLock ledgerLock = new ReentrantLock();
    private final long[] balances = new long[Contract.ACCOUNTS];
    private long appliedCount;

    Store(long openingBalance) {
        java.util.Arrays.fill(balances, openingBalance);
    }

    /** Handles one copy of a transfer and returns what the client is told. */
    Messages.Status handle(Messages.Transfer t, Mode mode) {
        long acc;
        if (mode.atomicClaim()) {
            if (applied.putIfAbsent(t.key(), CLAIMED) != null) {
                return Messages.Status.DUPLICATE;
            }
            acc = validate(mode.window());
            apply(t);
            applied.put(t.key(), t.amount());
        } else {
            if (applied.containsKey(t.key())) {
                return Messages.Status.DUPLICATE;
            }
            acc = validate(mode.window());
            apply(t);
            applied.put(t.key(), t.amount());
        }
        sink = acc;
        Hairspring.work(1);
        return Messages.Status.APPLIED;
    }

    private void apply(Messages.Transfer t) {
        ledgerLock.lock();
        try {
            balances[t.from()] -= t.amount();
            balances[t.to()] += t.amount();
            appliedCount++;
        } finally {
            ledgerLock.unlock();
        }
    }

    /** The busy loop standing in for request validation: plain arithmetic, no call, no allocation. */
    static long validate(int rounds) {
        long acc = SPIN_SEED;
        for (int i = 0; i < rounds; i++) {
            acc = (((acc << 7) | (acc >>> 57)) ^ i) * SPIN_MULTIPLIER;
        }
        return acc;
    }

    long appliedCount() {
        ledgerLock.lock();
        try {
            return appliedCount;
        } finally {
            ledgerLock.unlock();
        }
    }

    long totalBalance() {
        ledgerLock.lock();
        try {
            long sum = 0;
            for (long b : balances) {
                sum += b;
            }
            return sum;
        } finally {
            ledgerLock.unlock();
        }
    }
}
