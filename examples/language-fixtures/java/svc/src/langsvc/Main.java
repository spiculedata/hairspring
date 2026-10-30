package langsvc;

import hairspring.Hairspring;
import java.io.FileDescriptor;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.OutputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.util.Random;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.function.LongSupplier;

/**
 * The Java service fixture: {@code java -cp svc.jar langsvc.Main buggy|fixed|wide [--k=N]}. A small in-process payments
 * service: four clients send K transfers each, every transfer is delivered twice, a pool of three workers applies them through
 * an idempotency table. It reports {@code java.counter_exact} ({@code always}: the ledger applied exactly 4 x K transfers,
 * witness {@code [applied, 4*K, 4, K]}), {@code java.all_started} ({@code sometimes}: all four clients started) and three
 * {@code sometimes} sites (a reply timed out, a copy expired, the ticker ran), all evaluated once in the main thread after
 * the service has drained. Stdout gets the acceptance fixture's one line, in one write, so the evaluator reads
 * it unchanged: {@code LANGAT v1 lang=java mode=<mode> final=<20 digits> expect=<20 digits> order=<16 hex> rand=<16 hex>}.
 * Exit 0 whatever the property says; 3 if the program itself failed; 64 for a usage error.
 */
public final class Main {
    private static final String USAGE = "usage: java -cp svc.jar langsvc.Main buggy|fixed|wide [--k=N]";
    private static final long OPENING_BALANCE = 1_000_000L;

    private Main() {}

    public static void main(String[] args) {
        int status = run(args, new FileOutputStream(FileDescriptor.out), System.err, () -> new Random().nextLong());
        if (status != Contract.EXIT_OK) {
            Hairspring.exit(status);
        }
    }

    static int run(String[] args, OutputStream out, PrintStream err, LongSupplier rand) {
        Mode mode = args.length >= 1 ? Mode.parse(args[0]) : null;
        int k = args.length == 2 ? parseK(args[1]) : (args.length == 1 ? Contract.ITERATIONS : -1);
        if (mode == null || k < 1) {
            err.println(USAGE);
            return Contract.EXIT_USAGE;
        }
        Store store = new Store(OPENING_BALANCE);
        Server server = new Server(store, mode);
        Client[] clients = new Client[Contract.CLIENTS];
        int[] finished = new int[Contract.CLIENTS];
        AtomicInteger arrivals = new AtomicInteger();
        AtomicInteger started = new AtomicInteger();
        Thread[] threads = new Thread[Contract.CLIENTS];
        try {
            server.start();
            for (int i = 0; i < Contract.CLIENTS; i++) {
                int index = i;
                clients[i] = new Client(i, k, server);
                threads[i] = new Thread(() -> {
                    started.incrementAndGet();
                    clients[index].run();
                    finished[arrivals.getAndIncrement()] = index;
                }, "client-" + i);
                threads[i].start();
            }
            for (Thread thread : threads) {
                thread.join();
            }
            if (!server.drain()) {
                err.println("invariant failure: the service did not drain in time");
                return Contract.EXIT_INVARIANT;
            }
        } catch (InterruptedException interrupted) {
            Thread.currentThread().interrupt();
            err.println("invariant failure: interrupted");
            return Contract.EXIT_INVARIANT;
        }
        if (server.failure() != null) {
            err.println("invariant failure: " + server.failure());
            return Contract.EXIT_INVARIANT;
        }
        for (Client client : clients) {
            if (client.failure() != null) {
                err.println("invariant failure: " + client.failure());
                return Contract.EXIT_INVARIANT;
            }
            if (client.confirmed() != k) {
                err.println("invariant failure: a client confirmed " + client.confirmed() + " transfers of " + k);
                return Contract.EXIT_INVARIANT;
            }
        }
        if (store.totalBalance() != OPENING_BALANCE * Contract.ACCOUNTS) {
            err.println("invariant failure: money was created or destroyed");
            return Contract.EXIT_INVARIANT;
        }
        return report(mode, k, store, server, clients, finished, started.get(), out, err, rand);
    }

    private static int report(Mode mode, int k, Store store, Server server, Client[] clients, int[] finished, int started,
            OutputStream out, PrintStream err, LongSupplier rand) {
        long applied = store.appliedCount();
        long expected = (long) Contract.CLIENTS * k;
        long timeouts = 0;
        for (Client client : clients) {
            timeouts += client.timeouts();
        }
        Hairspring.always(applied == expected, Contract.LANG + ".counter_exact", "applied transfers equal clients x K", applied,
                expected, Contract.CLIENTS, k);
        Hairspring.sometimes(started == Contract.CLIENTS, Contract.LANG + ".all_started", "all clients started", started);
        Hairspring.sometimes(timeouts > 0, Contract.LANG + ".svc.reply_timeout", "a client timed out and resent", timeouts);
        Hairspring.sometimes(server.expired() > 0, Contract.LANG + ".svc.copy_expired", "a copy passed its deadline", server.expired());
        Hairspring.sometimes(server.ticks() > 0, Contract.LANG + ".svc.ticker_ran", "the metrics timer ran", server.ticks());
        String line = String.format("LANGAT v1 lang=java mode=%s final=%020d expect=%020d order=%016x rand=%016x%n", mode.label(),
                applied, expected, orderHash(finished), rand.getAsLong());
        byte[] bytes = line.getBytes(StandardCharsets.US_ASCII);
        try {
            out.write(bytes, 0, bytes.length);
            out.flush();
        } catch (IOException failed) {
            err.println("invariant failure: the stdout line could not be written: " + failed.getMessage());
            return Contract.EXIT_INVARIANT;
        }
        return Contract.EXIT_OK;
    }

    /** {@code --k=N}, 1 to MAX_ITERATIONS (5,000), digits only, no leading zero; -1 for anything else. */
    static int parseK(String arg) {
        if (!arg.startsWith("--k=")) {
            return -1;
        }
        String digits = arg.substring(4);
        if (digits.isEmpty() || digits.length() > 4 || digits.charAt(0) == '0' || !digits.chars().allMatch(c -> c >= '0' && c <= '9')) {
            return -1;
        }
        int k = Integer.parseInt(digits);
        return k <= Contract.MAX_ITERATIONS ? k : -1;
    }

    /** FNV-1a over the order the clients finished in. */
    static long orderHash(int[] order) {
        long h = 0xcbf29ce484222325L;
        for (int v : order) {
            h ^= v & 0xff;
            h *= 0x100000001b3L;
        }
        return h;
    }
}
