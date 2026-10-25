using System.Linq;

namespace Hairspring;

/// <summary>The five kinds (numbers are the C SDK's) and the catalog hash.</summary>
internal static class CatalogTests
{
    private static Catalog.Entry[] Basic() =>
    [
        new("basic.always.ok", Kind.Always),
        new("basic.always.bad", Kind.Always),
        new("basic.sometimes", Kind.Sometimes),
        new("basic.reach.hit", Kind.Reachable),
        new("basic.reach.never", Kind.Reachable),
        new("basic.unreach.clean", Kind.Unreachable),
        new("basic.aou.never", Kind.AlwaysOrUnreachable),
    ];

    private static readonly Kind[] AllKinds =
        [Kind.Always, Kind.AlwaysOrUnreachable, Kind.Sometimes, Kind.Reachable, Kind.Unreachable];

    public static void Register(Suite s)
    {
        s.Add("kind: the numbers are the C SDK's", () =>
            Check.Equal(new[] { 1, 2, 3, 4, 5 }, AllKinds.Select(KindInfo.Number).ToArray()));
        s.Add("kind: names and flags", () =>
        {
            Check.Equal("always_or_unreachable", KindInfo.WireName(Kind.AlwaysOrUnreachable));
            Check.Equal(new[] { true, false, true, true, false }, AllKinds.Select(KindInfo.MustHit).ToArray());
            Check.Equal(new[] { false, false, false, true, true }, AllKinds.Select(KindInfo.IsPoint).ToArray());
        });
        s.Add("kind: a value that is not a kind is refused", () =>
        {
            Check.Raises<System.ArgumentOutOfRangeException>(() => KindInfo.Check((Kind)0));
            Check.Raises<System.ArgumentOutOfRangeException>(() => KindInfo.Check((Kind)6));
            KindInfo.Check(Kind.Unreachable);
        });
        s.Add("catalog: the hash equals the C SDK's for the same ids and kinds", () =>
            // Printed by the C SDK's basic scenario: "catalog_hash":"fb6cf21a75aad6ef".
            Check.Equal(0xFB6CF21A75AAD6EFUL, Catalog.Hash(Basic())));
        s.Add("catalog: the hash does not depend on order", () =>
            Check.Equal(Catalog.Hash(Basic()), Catalog.Hash(Basic().Reverse())));
        s.Add("catalog: an id declared with two kinds joins both pairs to the hash", () =>
        {
            var one = Catalog.Hash([new Catalog.Entry("c", Kind.Always)]);
            var two = Catalog.Hash([new Catalog.Entry("c", Kind.Always), new Catalog.Entry("c", Kind.Sometimes)]);
            Check.No(one == two, "one pair and two pairs hash alike");
        });
        s.Add("catalog: an empty catalog hashes to zero", () => Check.Equal(0UL, Catalog.Hash([])));
        s.Add("catalog: distinct ids ignore the kind", () =>
            Check.Equal(2, Catalog.DistinctIds([
                new Catalog.Entry("c", Kind.Always),
                new Catalog.Entry("c", Kind.Sometimes),
                new Catalog.Entry("d", Kind.Reachable),
            ])));
    }
}
