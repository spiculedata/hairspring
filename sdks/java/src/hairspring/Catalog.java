package hairspring;

import java.util.Collection;
import java.util.HashSet;
import java.util.Set;

/**
 * The catalog's summary values: how many distinct ids, and the order-independent hash of the distinct (id, kind) pairs
 * that {@code hello.catalog_hash} carries. The hash equals the C SDK's for the same ids and kinds.
 */
final class Catalog {
    private static final int KIND_SHIFT = 56;
    private static final long HASH_MIX = 0x9E3779B97F4A7C15L;

    private Catalog() {}

    /** One distinct (id, kind) pair. */
    record Entry(String id, Kind kind) {}

    static long pairHash(Entry entry) {
        long mixed = Fmt.identHash(entry.id(), Fmt.ID_MAX) ^ ((long) entry.kind().number() << KIND_SHIFT);
        return mixed * HASH_MIX;
    }

    /** The sum of the pair hashes, so the order of the entries does not matter. Entries must be distinct pairs. */
    static long hash(Collection<Entry> entries) {
        long sum = 0;
        for (Entry entry : entries) {
            sum += pairHash(entry);
        }
        return sum;
    }

    static int distinctIds(Collection<Entry> entries) {
        Set<String> ids = new HashSet<>();
        for (Entry entry : entries) {
            ids.add(entry.id());
        }
        return ids.size();
    }
}
