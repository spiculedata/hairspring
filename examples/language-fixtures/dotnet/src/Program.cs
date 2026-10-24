using System;

namespace LangAt;

/// <summary>
/// <c>LangAtProbe.dll</c>: a BCL-only program used to check how .NET threads are scheduled. It has no SDK and no property,
/// and takes no arguments. Four threads take a lock once (the lock order), run an unlocked read-spin-write on a plain
/// counter, and queue their number when they finish (the arrival order); four tasks await a one millisecond delay and
/// queue their number (the completion order). It prints exactly one line on stdout, with one write, and exits 0:
/// <c>PROBE v1 lang=dotnet lock_order=&lt;16 hex&gt; arrival=&lt;16 hex&gt; rand=&lt;16 hex&gt; vtime_ns=&lt;20 digits&gt; extra=&lt;16 hex&gt;</c>
/// </summary>
internal static class Program
{
    private static int Main(string[] args)
    {
        try
        {
            using var stdout = Console.OpenStandardOutput();
            return Cli.Run(args, stdout, Console.Error);
        }
        catch (Exception e)
        {
            Console.Error.WriteLine($"probe-dotnet: internal invariant failed: unexpected {e.GetType().Name}: {e.Message}");
            return Cli.ExitInvariant;
        }
    }
}
