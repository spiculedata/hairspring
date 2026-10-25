using System.Linq;

namespace Hairspring;

/// <summary>Validation of what a call site passes, and the table that merges sites.</summary>
internal static class SitesTests
{
    public static void Register(Suite s)
    {
        s.Add("args: no values pack to four zeros", () => Check.Equal(new long[] { 0, 0, 0, 0 }, Args.PackVals([])));
        s.Add("args: values are zero filled to four", () => Check.Equal(new long[] { 7, 8, 0, 0 }, Args.PackVals([7, 8])));
        s.Add("args: a fifth value is refused", () => Check.Raises<System.ArgumentException>(() => Args.PackVals([1, 2, 3, 4, 5])));
        s.Add("args: an id is 1 to 128 bytes", () =>
        {
            Args.CheckId("a");
            Args.CheckId(new string('x', 128));
            Check.Raises<System.ArgumentException>(() => Args.CheckId(""));
            Check.Raises<System.ArgumentException>(() => Args.CheckId(new string('x', 129)));
            Check.Raises<System.ArgumentException>(() => Args.CheckId(new string('\u00e9', 65)));
            Check.Raises<System.ArgumentNullException>(() => Args.CheckId(null!));
        });
        s.Add("args: an id with a NUL is refused", () => Check.Raises<System.ArgumentException>(() => Args.CheckId("a\0b")));
        s.Add("args: a message is at most 512 bytes", () =>
        {
            Args.CheckMsg(new string('m', 512));
            Check.Raises<System.ArgumentException>(() => Args.CheckMsg(new string('m', 513)));
            Check.Raises<System.ArgumentNullException>(() => Args.CheckMsg(null!));
        });
        s.Add("args: the message limit counts UTF-8 bytes", () =>
        {
            Args.CheckMsg(new string('\u20ac', 170));
            Check.Raises<System.ArgumentException>(() => Args.CheckMsg(new string('\u20ac', 171)));
        });
        s.Add("sites: the same id and kind is one site, the first registration stands", () =>
        {
            var table = new SiteTable();
            var a = table.Register(Kind.Always, "x", "first", "A.cs", 1)!;
            var b = table.Register(Kind.Always, "x", "second", "B.cs", 2)!;
            Check.Yes(ReferenceEquals(a, b), "one site");
            Check.Equal("first", a.Msg);
            Check.Equal("A.cs", a.File);
            Check.Equal(1, table.DistinctIds);
        });
        s.Add("sites: ids that differ only outside the alphabet are one id", () =>
        {
            var table = new SiteTable();
            var a = table.Register(Kind.Always, "a b", "", "A.cs", 1)!;
            var b = table.Register(Kind.Always, "a_b", "", "A.cs", 2)!;
            Check.Yes(ReferenceEquals(a, b), "one site");
            Check.Equal("a_b", a.Id);
            Check.Yes(ReferenceEquals(table.Find(Kind.Always, "a b"), a), "the caller's spelling finds it");
        });
        s.Add("sites: one id with two kinds is two sites and one distinct id", () =>
        {
            var table = new SiteTable();
            var a = table.Register(Kind.Always, "c", "", "A.cs", 1)!;
            var b = table.Register(Kind.Sometimes, "c", "", "A.cs", 2)!;
            Check.No(ReferenceEquals(a, b), "two sites");
            Check.Equal(2, table.All().Count);
            Check.Equal(1, table.DistinctIds);
            Check.Equal(new[] { a }, table.Canonical().ToArray());
        });
        s.Add("sites: find is empty before a registration", () => Check.Yes(new SiteTable().Find(Kind.Always, "nope") is null, "no site"));
        s.Add("sites: a full table refuses a new id and remembers that it did", () =>
        {
            var table = new SiteTable();
            for (var i = 0; i < SiteTable.MaxDistinct; i++)
            {
                Check.Yes(table.Register(Kind.Reachable, "id" + i, "", "A.cs", 1) is not null, "room for " + i);
            }
            Check.Yes(table.Register(Kind.Reachable, "one-too-many", "", "A.cs", 1) is null, "refused");
            Check.Yes(ReferenceEquals(table.Find(Kind.Reachable, "one-too-many"), Site.Untracked), "remembered as untracked");
            Check.Yes(table.Register(Kind.Sometimes, "id0", "", "A.cs", 1) is not null, "a known id can take another kind");
        });
        s.Add("sites: the spellings the lookup remembers are bounded", () =>
        {
            var table = new SiteTable();
            for (var i = 0; i < SiteTable.MaxRemembered + 100; i++)
            {
                table.Register(Kind.Reachable, "same", "", "A.cs", 1);
                table.Register(Kind.Reachable, "same" + (i % 3 == 0 ? " " : "_") + i, "", "A.cs", 1);
                if (table.DistinctIds >= SiteTable.MaxDistinct)
                {
                    break;
                }
            }
            Check.Yes(table.RememberedCount <= SiteTable.MaxRemembered, "remembered " + table.RememberedCount);
        });
        s.Add("sites: registration validates the id and the message", () =>
        {
            var table = new SiteTable();
            Check.Raises<System.ArgumentException>(() => table.Register(Kind.Always, "", "", "A.cs", 1));
            Check.Raises<System.ArgumentException>(() => table.Register(Kind.Always, "x", new string('m', 513), "A.cs", 1));
            Check.Equal(0, table.All().Count);
        });
    }
}
