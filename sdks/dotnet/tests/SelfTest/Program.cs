using System;

namespace Hairspring;

/// <summary>The SDK's own tests: no framework, one process. Exits 0 when every case passed, 1 otherwise. An argument filters by name.</summary>
internal static class Program
{
    private static Suite Build()
    {
        var suite = new Suite();
        FmtTests.Register(suite);
        CatalogTests.Register(suite);
        LinesTests.Register(suite);
        IdentityTests.Register(suite);
        SlotsTests.Register(suite);
        SitesTests.Register(suite);
        SecureTests.Register(suite);
        NativeTests.Register(suite);
        OutputTests.Register(suite);
        RecorderTests.Register(suite);
        return suite;
    }

    private static int Main(string[] args)
    {
        // An argument that does not start with a dash is a filter on the case names (options such as --nologo are ignored).
        var filter = Array.Find(args, a => !a.StartsWith('-'));
        var failed = Build().Run(Console.Out, filter);
        return failed == 0 ? 0 : 1;
    }
}
