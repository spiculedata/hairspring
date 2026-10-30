package langsvc;

import hairspring.Hairspring;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;

/**
 * One client: K transfers, one after another. Each is sent as two copies at once (at-least-once delivery); the client then
 * waits for a reply with a timeout. On a timeout or an EXPIRED reply it sends one more copy with a doubled time to live
 * (a clock-dependent retry). An APPLIED or DUPLICATE reply for the current key ends the transfer; replies for earlier keys
 * (the late copies) are dropped.
 */
final class Client implements Runnable {
    private final int index;
    private final int iterations;
    private final Server server;
    private final BlockingQueue<Messages.Reply> replies = new LinkedBlockingQueue<>();
    private long timeouts;
    private long retries;
    private long confirmed;
    private volatile String failure;

    Client(int index, int iterations, Server server) {
        this.index = index;
        this.iterations = iterations;
        this.server = server;
    }

    @Override
    public void run() {
        try {
            Hairspring.thread("client", index);
            for (int k = 0; k < iterations; k++) {
                transfer(k);
            }
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            failure = "client " + index + " interrupted";
        } catch (RuntimeException | Error problem) {
            failure = "client " + index + ": " + problem;
        }
    }

    private void transfer(int k) throws InterruptedException {
        long key = Messages.key(index, k);
        Messages.Transfer t = new Messages.Transfer(key, k % Contract.ACCOUNTS, (k + index + 1) % Contract.ACCOUNTS, 1 + k % 5);
        String payload = Messages.encode(t);
        long ttl = Contract.TTL_MS;
        for (int copy = 0; copy < Contract.COPIES; copy++) {
            server.deliver(new Messages.Request(payload, Deadline.after(ttl), replies));
        }
        int attempts = Contract.COPIES;
        while (true) {
            Messages.Reply reply = replies.poll(Contract.REPLY_TIMEOUT_MS, TimeUnit.MILLISECONDS);
            if (reply != null && reply.status() == Messages.Status.FAILED) {
                throw new IllegalStateException("the service failed a copy of transfer " + k);
            }
            if (reply == null) {
                timeouts++;
            } else if (reply.key() != key) {
                continue;
            } else if (reply.status() != Messages.Status.EXPIRED) {
                if (reply.key() == key) {
                    confirmed++;
                }
                return;
            }
            if (attempts++ >= Contract.MAX_ATTEMPTS) {
                throw new IllegalStateException("gave up on transfer " + k + " after " + Contract.MAX_ATTEMPTS + " copies");
            }
            retries++;
            ttl *= 2;
            server.deliver(new Messages.Request(payload, Deadline.after(ttl), replies));
        }
    }

    long timeouts() {
        return timeouts;
    }

    /** Transfers that ended on a reply for their own key; the main thread requires K per client. */
    long confirmed() {
        return confirmed;
    }

    long retries() {
        return retries;
    }

    String failure() {
        return failure;
    }
}
