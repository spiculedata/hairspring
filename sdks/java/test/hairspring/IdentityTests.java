package hairspring;

/** The process tag and how it survives a spawn. */
final class IdentityTests {
    private IdentityTests() {}

    private static Identity id(String tag, boolean stable, boolean refused) {
        return new Identity(tag, stable, refused);
    }

    static void register(Suite s) {
        s.add("identity: without a harness tag the root process is p1", () -> {
            Check.equal(id("p1", true, false), Identity.resolve(null, null, 10));
            Check.equal(id("p1", true, false), Identity.resolve("", null, 10));
        });
        s.add("identity: a harness tag without an owner is the root tag", () ->
                Check.equal(id("job-7", true, false), Identity.resolve("job-7", null, 10)));
        s.add("identity: an invalid harness tag is refused, not shortened", () -> {
            for (String bad : new String[] {"_eval", "a b", "x".repeat(65)}) {
                Check.equal(bad, id("p1", true, true), Identity.resolve(bad, null, 10));
            }
        });
        s.add("identity: tag validity at its limits", () -> {
            Check.yes(Identity.tagValid("a"), "a");
            Check.yes(Identity.tagValid("x".repeat(64)), "64 bytes");
            Check.no(Identity.tagValid(""), "empty");
            Check.no(Identity.tagValid("_eval"), "_eval");
            Check.no(Identity.tagValid("a#b"), "a#b");
        });
        s.add("identity: a child whose inherited pid is not its own takes a spawn tag", () -> {
            Check.equal(id("p1.s42", false, false), Identity.resolve("p1", "999", 42));
            Check.equal(id("p1.s42", false, false), Identity.resolve("p1", "junk", 42));
            Check.equal(id("p1.s42", false, false), Identity.resolve("p1", "99999999999999999999999", 42));
        });
        s.add("identity: an image that owns the inherited pid keeps the tag", () ->
                Check.equal(id("p1", true, false), Identity.resolve("p1", "42", 42)));
        s.add("identity: an exec without fork after the report takes an x tag", () ->
                Check.equal(id("p1.x1", true, false), Identity.resolve("p1", "42:a", 42)));
        s.add("identity: an unstable tag stays unstable across an exec", () -> {
            Check.equal(id("p1.s9", false, false), Identity.resolve("p1.s9", "42:u", 42));
            Check.equal(id("p1.s9.x1", false, false), Identity.resolve("p1.s9", "42:au", 42));
        });
        s.add("identity: the owner field records the pid and its flags", () -> {
            Check.equal("42", Identity.ownerField(42, false, true));
            Check.equal("42:a", Identity.ownerField(42, true, true));
            Check.equal("42:u", Identity.ownerField(42, false, false));
            Check.equal("42:au", Identity.ownerField(42, true, false));
        });
        s.add("identity: derived tags append a letter and a number", () ->
                Check.equal("p1.f2", Identity.derive("p1", 'f', 2)));
        s.add("identity: a derived tag that would pass 64 bytes keeps a prefix and a hash", () -> {
            String base = "b".repeat(62);
            String tag = Identity.derive(base, 's', 123456);
            Check.yes(tag.length() <= 64, tag);
            Check.yes(tag.startsWith("b".repeat(40)), tag);
            Check.yes(tag.contains(".h"), tag);
            Check.yes(Identity.tagValid(tag), tag);
            Check.no(tag.equals(Identity.derive(base, 's', 123457)), "different pids give different tags");
        });
    }
}
