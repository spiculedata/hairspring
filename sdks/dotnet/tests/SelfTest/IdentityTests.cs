namespace Hairspring;

/// <summary>The process tag and how it survives a spawn.</summary>
internal static class IdentityTests
{
    private static Identity Id(string tag, bool stable, bool refused) => new(tag, stable, refused);

    public static void Register(Suite s)
    {
        s.Add("identity: without a harness tag the root process is p1", () =>
        {
            Check.Equal(Id("p1", true, false), Identity.Resolve(null, null, 10));
            Check.Equal(Id("p1", true, false), Identity.Resolve("", null, 10));
        });
        s.Add("identity: a harness tag without an owner is the root tag", () =>
            Check.Equal(Id("job-7", true, false), Identity.Resolve("job-7", null, 10)));
        s.Add("identity: an invalid harness tag is refused, not shortened", () =>
        {
            foreach (var bad in new[] { "_eval", "a b", new string('x', 65), "caf\u00e9" })
            {
                Check.Equal(bad, Id("p1", true, true), Identity.Resolve(bad, null, 10));
            }
        });
        s.Add("identity: tag validity at its limits", () =>
        {
            Check.Yes(Identity.TagValid("a"), "a");
            Check.Yes(Identity.TagValid(new string('x', 64)), "64 bytes");
            Check.No(Identity.TagValid(""), "empty");
            Check.No(Identity.TagValid("_eval"), "_eval");
            Check.No(Identity.TagValid("a#b"), "a#b");
        });
        s.Add("identity: a child whose inherited pid is not its own takes a spawn tag", () =>
        {
            Check.Equal(Id("p1.s42", false, false), Identity.Resolve("p1", "999", 42));
            Check.Equal(Id("p1.s42", false, false), Identity.Resolve("p1", "junk", 42));
            Check.Equal(Id("p1.s42", false, false), Identity.Resolve("p1", "99999999999999999999999", 42));
        });
        s.Add("identity: an image that owns the inherited pid keeps the tag", () =>
            Check.Equal(Id("p1", true, false), Identity.Resolve("p1", "42", 42)));
        s.Add("identity: an exec without fork after the report takes an x tag", () =>
            Check.Equal(Id("p1.x1", true, false), Identity.Resolve("p1", "42:a", 42)));
        s.Add("identity: an unstable tag stays unstable across an exec", () =>
        {
            Check.Equal(Id("p1.s9", false, false), Identity.Resolve("p1.s9", "42:u", 42));
            Check.Equal(Id("p1.s9.x1", false, false), Identity.Resolve("p1.s9", "42:au", 42));
        });
        s.Add("identity: the owner field records the pid and its flags", () =>
        {
            Check.Equal("42", Identity.OwnerField(42, false, true));
            Check.Equal("42:a", Identity.OwnerField(42, true, true));
            Check.Equal("42:u", Identity.OwnerField(42, false, false));
            Check.Equal("42:au", Identity.OwnerField(42, true, false));
        });
        s.Add("identity: derived tags append a letter and a number", () =>
            Check.Equal("p1.f2", Identity.Derive("p1", 'f', 2)));
        s.Add("identity: a derived tag that would pass 64 bytes keeps a prefix and a hash", () =>
        {
            var baseTag = new string('b', 62);
            var tag = Identity.Derive(baseTag, 's', 123456);
            Check.Yes(tag.Length <= 64, tag);
            Check.Yes(tag.StartsWith(new string('b', 40), System.StringComparison.Ordinal), tag);
            Check.Yes(tag.Contains(".h", System.StringComparison.Ordinal), tag);
            Check.Yes(Identity.TagValid(tag), tag);
            Check.No(tag == Identity.Derive(baseTag, 's', 123457), "different pids give different tags");
        });
    }
}
