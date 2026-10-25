using System;
using Hairspring;

namespace HsSample;

/// <summary>
/// Sample instrumented programs, one per scenario: <c>dotnet Sample.dll &lt;scenario&gt; [args]</c>.
/// <para>The process tests (test/test-process.sh) and the Rust conformance test (crates/hairspring-props/tests/dotnet_sdk.rs)
/// run them and inspect the events they write. <c>basic</c> has the same ids, messages, values and order as the C SDK's
/// <c>basic</c> scenario (sdks/c/tests/fixtures/hs_sample.c), so the two SDKs' reports can be compared line by line; the three
/// sites the C program never reaches are declared here, because .NET cannot see a call site that has not run.</para>
/// </summary>
internal static class Program
{
    private const int ExitUsage = 64;

    private static int Main(string[] args)
    {
        if (args.Length < 1 || !System.Linq.Enumerable.Contains(Scenarios.Names, args[0]))
        {
            Console.Error.WriteLine("usage: Sample <" + string.Join("|", Scenarios.Names) + "> [args]");
            Hs.Exit(ExitUsage);
        }
        return Scenarios.Run(args[0], args[1..]);
    }
}
