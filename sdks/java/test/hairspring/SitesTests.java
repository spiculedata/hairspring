package hairspring;

import java.util.List;

/** Validation of what a call site passes, and the table that merges sites. */
final class SitesTests {
    private SitesTests() {}

    static void register(Suite s) {
        s.add("args: no values pack to four zeros", () -> Check.equal(new long[] {0, 0, 0, 0}, Args.packVals(new long[0])));
        s.add("args: values are zero filled to four", () -> Check.equal(new long[] {7, 8, 0, 0}, Args.packVals(new long[] {7, 8})));
        s.add("args: a fifth value is refused", () ->
                Check.raises(IllegalArgumentException.class, () -> Args.packVals(new long[] {1, 2, 3, 4, 5})));
        s.add("args: an id is 1 to 128 bytes", () -> {
            Args.checkId("a");
            Args.checkId("x".repeat(128));
            Check.raises(IllegalArgumentException.class, () -> Args.checkId(""));
            Check.raises(IllegalArgumentException.class, () -> Args.checkId("x".repeat(129)));
            Check.raises(IllegalArgumentException.class, () -> Args.checkId("é".repeat(65)));
            Check.raises(NullPointerException.class, () -> Args.checkId(null));
        });
        s.add("args: an id with a NUL is refused", () ->
                Check.raises(IllegalArgumentException.class, () -> Args.checkId("a\0b")));
        s.add("args: a message is at most 512 bytes", () -> {
            Args.checkMsg("m".repeat(512));
            Check.raises(IllegalArgumentException.class, () -> Args.checkMsg("m".repeat(513)));
            Check.raises(NullPointerException.class, () -> Args.checkMsg(null));
        });
        s.add("sites: the same id and kind is one site, the first registration stands", () -> {
            SiteTable table = new SiteTable();
            Site a = table.register(Kind.ALWAYS, "x", "first", "A.java", 1);
            Site b = table.register(Kind.ALWAYS, "x", "second", "B.java", 2);
            Check.yes(a == b, "one site");
            Check.equal("first", a.msg);
            Check.equal("A.java", a.file);
            Check.equal(1, table.distinctIds());
        });
        s.add("sites: ids that differ only outside the alphabet are one id", () -> {
            SiteTable table = new SiteTable();
            Site a = table.register(Kind.ALWAYS, "a b", "", "A.java", 1);
            Site b = table.register(Kind.ALWAYS, "a_b", "", "A.java", 2);
            Check.yes(a == b, "one site");
            Check.equal("a_b", a.id);
            Check.yes(table.find(Kind.ALWAYS, "a b") == a, "the caller's spelling finds it");
        });
        s.add("sites: one id with two kinds is two sites and one distinct id", () -> {
            SiteTable table = new SiteTable();
            Site a = table.register(Kind.ALWAYS, "c", "", "A.java", 1);
            Site b = table.register(Kind.SOMETIMES, "c", "", "A.java", 2);
            Check.no(a == b, "two sites");
            Check.equal(2, table.all().size());
            Check.equal(1, table.distinctIds());
            Check.equal(List.of(a), table.canonical());
        });
        s.add("sites: find is empty before a registration", () ->
                Check.yes(new SiteTable().find(Kind.ALWAYS, "nope") == null, "no site"));
        s.add("sites: a full table refuses a new id and remembers that it did", () -> {
            SiteTable table = new SiteTable();
            for (int i = 0; i < SiteTable.MAX_DISTINCT; i++) {
                Check.yes(table.register(Kind.REACHABLE, "id" + i, "", "A.java", 1) != null, "room for " + i);
            }
            Check.yes(table.register(Kind.REACHABLE, "one-too-many", "", "A.java", 1) == null, "refused");
            Check.yes(table.find(Kind.REACHABLE, "one-too-many") == Site.UNTRACKED, "remembered as untracked");
            Check.yes(table.register(Kind.SOMETIMES, "id0", "", "A.java", 1) != null, "a known id can take another kind");
        });
        s.add("sites: registration validates the id and the message", () -> {
            SiteTable table = new SiteTable();
            Check.raises(IllegalArgumentException.class, () -> table.register(Kind.ALWAYS, "", "", "A.java", 1));
            Check.raises(IllegalArgumentException.class, () -> table.register(Kind.ALWAYS, "x", "m".repeat(513), "A.java", 1));
            Check.equal(0, table.all().size());
        });
    }
}
