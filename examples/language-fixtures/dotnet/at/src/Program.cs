using System;

namespace LangAt;

/// <summary>
/// <c>LangAt.dll</c>: the .NET lost-update fixture. One program with three modes chosen by its first argument (<c>buggy</c>,
/// <c>fixed</c>, <c>wide</c>); see <see cref="Cli"/>. The Hairspring .NET SDK (<c>Hairspring.dll</c>, beside it) reports the
/// properties when the process exits.
/// </summary>
internal static class Program
{
    private static int Main(string[] args)
    {
        try
        {
            return Cli.Run(args, Stdout.Write, Console.Error, mode => Race.Run(mode, Contract.Iterations), () => (ulong)Random.Shared.NextInt64());
        }
        catch (Exception e)
        {
            Console.Error.WriteLine($"invariant failure: unexpected {e.GetType().Name}: {e.Message}");
            return Contract.ExitInvariant;
        }
    }
}
