package hairspring;

import java.util.List;

/** The five kinds (numbers are the C SDK's) and the catalog hash. */
final class CatalogTests {
    private CatalogTests() {}

    private static List<Catalog.Entry> basic() {
        return List.of(
                new Catalog.Entry("basic.always.ok", Kind.ALWAYS),
                new Catalog.Entry("basic.always.bad", Kind.ALWAYS),
                new Catalog.Entry("basic.sometimes", Kind.SOMETIMES),
                new Catalog.Entry("basic.reach.hit", Kind.REACHABLE),
                new Catalog.Entry("basic.reach.never", Kind.REACHABLE),
                new Catalog.Entry("basic.unreach.clean", Kind.UNREACHABLE),
                new Catalog.Entry("basic.aou.never", Kind.ALWAYS_OR_UNREACHABLE));
    }

    static void register(Suite s) {
        s.add("kind: the numbers are the C SDK's", () -> {
            Check.equal(List.of(1, 2, 3, 4, 5), List.of(Kind.values()).stream().map(Kind::number).toList());
        });
        s.add("kind: names and flags", () -> {
            Check.equal("always_or_unreachable", Kind.ALWAYS_OR_UNREACHABLE.wireName());
            Check.equal(
                    List.of(true, false, true, true, false),
                    List.of(Kind.values()).stream().map(Kind::mustHit).toList());
            Check.equal(
                    List.of(false, false, false, true, true),
                    List.of(Kind.values()).stream().map(Kind::isPoint).toList());
        });
        s.add("catalog: the hash equals the C SDK's for the same ids and kinds", () -> {
            // Printed by the C SDK's basic scenario: "catalog_hash":"fb6cf21a75aad6ef".
            Check.equal(0xFB6CF21A75AAD6EFL, Catalog.hash(basic()));
        });
        s.add("catalog: the hash does not depend on order", () -> {
            Check.equal(Catalog.hash(basic()), Catalog.hash(basic().reversed()));
        });
        s.add("catalog: an id declared with two kinds joins both pairs to the hash", () -> {
            long one = Catalog.hash(List.of(new Catalog.Entry("c", Kind.ALWAYS)));
            long two = Catalog.hash(List.of(
                    new Catalog.Entry("c", Kind.ALWAYS), new Catalog.Entry("c", Kind.SOMETIMES)));
            Check.no(one == two, "one pair and two pairs hash alike");
        });
        s.add("catalog: an empty catalog hashes to zero", () -> Check.equal(0L, Catalog.hash(List.of())));
        s.add("catalog: distinct ids ignore the kind", () -> {
            Check.equal(2, Catalog.distinctIds(List.of(
                    new Catalog.Entry("c", Kind.ALWAYS),
                    new Catalog.Entry("c", Kind.SOMETIMES),
                    new Catalog.Entry("d", Kind.REACHABLE))));
        });
    }
}
