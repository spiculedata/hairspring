package hairspring;

/**
 * The process tag, and how it survives a spawn.
 *
 * <p>The tag travels in the environment as {@code HAIRSPRING_PROC}, with the owning pid and two flags in
 * {@code HAIRSPRING_PROC_PID} ({@code <pid>}, {@code <pid>:a}, {@code <pid>:u}, {@code <pid>:au}; {@code a} = the tag has
 * been announced with a report, {@code u} = the tag is not stable across runs):
 *
 * <ul>
 *   <li>root process: {@code HAIRSPRING_PROC} if the harness set it, else {@code p1};
 *   <li>spawned child: the inherited pid is not its own, so the inherited tag belongs to an ancestor and the child takes
 *       {@code <inherited>.s<pid>}, flagged unstable (a pid differs between runs);
 *   <li>an image that owns the inherited pid and whose tag was announced takes {@code <tag>.x1} (Java cannot exec, so
 *       this only matters to a native launcher that execs a JVM).
 * </ul>
 *
 * <p>There is no fork handler: Java cannot fork.
 */
record Identity(String tag, boolean tagStable, boolean refused) {
    static final String ENV_TAG = "HAIRSPRING_PROC";
    static final String ENV_PID = "HAIRSPRING_PROC_PID";
    static final String DEFAULT_TAG = "p1";
    static final String RESERVED_TAG = "_eval";
    /** The readable part kept when a derived tag would pass 64 bytes. */
    private static final int HASH_PREFIX_MAX = 40;
    private static final long HASH_MIX = 0x9E3779B97F4A7C15L;
    private static final long NO_PID = -1;

    /** 1 to 64 bytes of {@code [A-Za-z0-9._:/-]}, and not {@code _eval}, which is reserved for {@code eval} lines. */
    static boolean tagValid(String tag) {
        byte[] data = Fmt.utf8(tag);
        if (data.length == 0 || data.length > Fmt.TAG_MAX || RESERVED_TAG.equals(tag)) {
            return false;
        }
        for (byte b : data) {
            if (!Fmt.isIdentChar(b & 0xff)) {
                return false;
            }
        }
        return true;
    }

    /** {@code base.<letter><n>}, or {@code <first 40 bytes of base>.h<16 hex digits>} when that would not fit in a tag. */
    static String derive(String base, char letter, long n) {
        String plain = base + "." + letter + n;
        if (plain.length() <= Fmt.TAG_MAX) {
            return plain;
        }
        int keep = Math.min(base.length(), HASH_PREFIX_MAX);
        long mix = Fmt.identHash(base, Fmt.TAG_MAX) ^ (n * HASH_MIX) ^ letter;
        return base.substring(0, keep) + ".h" + Fmt.hex64(mix);
    }

    /** The value of {@code HAIRSPRING_PROC_PID}. */
    static String ownerField(long pid, boolean announced, boolean stable) {
        if (!announced && stable) {
            return Long.toString(pid);
        }
        return pid + ":" + (announced ? "a" : "") + (stable ? "" : "u");
    }

    private record Owner(long pid, boolean announced, boolean unstable) {}

    private static boolean isAsciiDigit(char c) {
        return c >= '0' && c <= '9';
    }

    /** {@code <pid>} or {@code <pid>:} followed by the flags {@code a} and {@code u}; anything else is not an owner field. */
    private static Owner parseOwner(String text) {
        int digits = 0;
        while (digits < text.length() && isAsciiDigit(text.charAt(digits))) {
            digits++;
        }
        if (digits == 0) {
            return null;
        }
        long pid;
        try {
            pid = Long.parseLong(text.substring(0, digits));
        } catch (NumberFormatException tooBig) {
            pid = NO_PID; // a number no pid equals; the field is still well formed
        }
        String rest = text.substring(digits);
        if (rest.isEmpty()) {
            return new Owner(pid, false, false);
        }
        if (rest.charAt(0) != ':') {
            return null;
        }
        for (int i = 1; i < rest.length(); i++) {
            if (rest.charAt(i) != 'a' && rest.charAt(i) != 'u') {
                return null;
            }
        }
        return new Owner(pid, rest.indexOf('a') >= 0, rest.indexOf('u') >= 0);
    }

    /** Decide the tag from the inherited environment values and this process's pid. */
    static Identity resolve(String inheritedTag, String owner, long pid) {
        if (inheritedTag == null || inheritedTag.isEmpty()) {
            return new Identity(DEFAULT_TAG, true, false);
        }
        if (!tagValid(inheritedTag)) {
            return new Identity(DEFAULT_TAG, true, true);
        }
        if (owner == null) {
            return new Identity(inheritedTag, true, false);
        }
        Owner parsed = parseOwner(owner);
        if (parsed != null && parsed.pid() == pid) {
            String tag = parsed.announced() ? derive(inheritedTag, 'x', 1) : inheritedTag;
            return new Identity(tag, !parsed.unstable(), false);
        }
        return new Identity(derive(inheritedTag, 's', pid), false, false);
    }
}
