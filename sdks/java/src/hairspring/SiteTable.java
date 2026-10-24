package hairspring;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Assertion sites. A site is one (id, kind) pair: the same pair evaluated from several places is one site, so counts
 * merge by construction; an id used with two kinds is two sites (a catalog conflict the engine reports).
 *
 * <p>{@link #register}, {@link #all}, {@link #canonical} and {@link #distinctIds} are called with the recorder's lock
 * held. {@link #find} is a single concurrent-map read and is safe without it.
 */
final class SiteTable {
    /** What the catalog can hold; ids beyond it are refused and the caller counts them as dropped. */
    static final int MAX_DISTINCT = 16384;
    /** How many spellings (as the caller wrote them) the lookup remembers, sites and refused ids together. */
    static final int MAX_REMEMBERED = 2 * MAX_DISTINCT;

    private record Key(String sent, Kind kind) {}

    /** The hot path's lookup, one map per kind, keyed by the id as the caller wrote it: no lock and no key object. */
    private final List<Map<String, Site>> raw = new ArrayList<>();

    private final Map<Key, Site> bySent = new HashMap<>();
    private final Set<String> ids = new HashSet<>();
    private final List<Site> order = new ArrayList<>();
    private int remembered;

    SiteTable() {
        for (int i = 0; i < Kind.values().length; i++) {
            raw.add(new ConcurrentHashMap<>());
        }
    }

    Site find(Kind kind, String rawId) {
        return raw.get(kind.ordinal()).get(rawId);
    }

    /**
     * The site for this id and kind, made if it is new. {@code null} when the table is full of other ids. The first
     * registration's message and location stand.
     */
    Site register(Kind kind, String rawId, String msg, String file, long line) {
        Args.checkId(rawId);
        Args.checkMsg(msg);
        String sent = Fmt.sanitizeIdent(rawId, Fmt.ID_MAX);
        Site existing = bySent.get(new Key(sent, kind));
        if (existing != null) {
            remember(rawId, kind, existing);
            return existing;
        }
        if (!ids.contains(sent) && ids.size() >= MAX_DISTINCT) {
            remember(rawId, kind, Site.UNTRACKED);
            return null;
        }
        Site site = new Site(sent, kind, msg, file, line);
        bySent.put(new Key(sent, kind), site);
        remember(rawId, kind, site);
        ids.add(sent);
        order.add(site);
        return site;
    }

    /** Cache a spelling for the lock-free lookup. Past the bound it is not cached: that call takes the slow path again. */
    private void remember(String rawId, Kind kind, Site site) {
        if (remembered < MAX_REMEMBERED) {
            raw.get(kind.ordinal()).put(rawId, site);
            remembered++;
        }
    }

    /** Every site in registration order. */
    List<Site> all() {
        return List.copyOf(order);
    }

    /** The first site of each id: the one whose counts the {@code assert} line carries. */
    List<Site> canonical() {
        Set<String> seen = new HashSet<>();
        List<Site> first = new ArrayList<>();
        for (Site site : order) {
            if (seen.add(site.id)) {
                first.add(site);
            }
        }
        return List.copyOf(first);
    }

    int distinctIds() {
        return ids.size();
    }
}
