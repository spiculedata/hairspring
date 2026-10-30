package langsvc;

import java.util.concurrent.BlockingQueue;

/** The wire types: a transfer request (encoded as text, as a real client would send it) and the reply. */
final class Messages {
    private Messages() {}

    enum Status { APPLIED, DUPLICATE, EXPIRED, FAILED }

    record Reply(long key, Status status) {}

    /** One delivered copy of a transfer. The payload is the text the worker parses; the deadline is on the monotonic clock. */
    record Request(String payload, long deadline, BlockingQueue<Reply> replyTo) {}

    record Transfer(long key, int from, int to, long amount) {}

    static String encode(Transfer t) {
        return "transfer key=" + t.key() + " from=" + t.from() + " to=" + t.to() + " amount=" + t.amount();
    }

    /** Parses what {@link #encode} wrote; anything else is a bug in this program, so it throws. */
    static Transfer decode(String payload) {
        String[] parts = payload.split(" ");
        if (parts.length != 5 || !parts[0].equals("transfer")) {
            throw new IllegalStateException("bad payload: " + payload);
        }
        return new Transfer(field(parts[1], "key"), (int) field(parts[2], "from"), (int) field(parts[3], "to"), field(parts[4], "amount"));
    }

    private static long field(String part, String name) {
        if (!part.startsWith(name + "=")) {
            throw new IllegalStateException("bad field " + part + ", wanted " + name);
        }
        return Long.parseLong(part.substring(name.length() + 1));
    }

    /** The idempotency key of client {@code client}'s {@code k}-th transfer. */
    static long key(int client, int k) {
        return ((long) client << 32) | k;
    }
}
