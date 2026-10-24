using System;
using System.IO;
using System.Text;

namespace LangAt;

/// <summary>
/// The command line of the probe: it takes no arguments, runs the workload and writes the one stdout line with a single
/// write. Exit 0 on success, 3 for an internal invariant failure (a worker did not start or finish, the line could not be
/// written), 64 for a usage error (any argument).
/// </summary>
internal static class Cli
{
    public const int ExitOk = 0;
    public const int ExitInvariant = 3;
    public const int ExitUsage = 64;
    public const string Usage = "usage: dotnet LangAtProbe.dll";

    /// <summary>The six values of the line for one run.</summary>
    public static Fields FieldsOf(Outcome outcome) => new(
        Line.HashDigits(outcome.LockOrder),
        Line.HashDigits(outcome.Arrival),
        outcome.Rand,
        outcome.ElapsedNs,
        Line.HashDigits(outcome.CompletionOrder));

    /// <summary>Runs the probe: nothing is written to <paramref name="stdout"/> unless the whole run succeeded.</summary>
    public static int Run(string[] args, Stream stdout, TextWriter stderr)
    {
        if (args.Length > 0)
        {
            stderr.WriteLine(Usage);
            return ExitUsage;
        }
        try
        {
            var outcome = Workload.Run();
            WriteLine(stdout, Line.Format(FieldsOf(outcome)));
            return ExitOk;
        }
        catch (InvariantException e)
        {
            stderr.WriteLine($"probe-dotnet: internal invariant failed: {e.Message}{CauseOf(e)}");
            return ExitInvariant;
        }
        catch (IOException e)
        {
            stderr.WriteLine($"probe-dotnet: internal invariant failed: the stdout line could not be written: {e.Message}");
            return ExitInvariant;
        }
    }

    /// <summary>The whole line in one call to Write (the line is ASCII, so its bytes are its characters), then a flush.</summary>
    private static void WriteLine(Stream stdout, string text)
    {
        stdout.Write(Encoding.ASCII.GetBytes(text));
        stdout.Flush();
    }

    private static string CauseOf(Exception e) => e.InnerException is null ? string.Empty : $" ({e.InnerException.GetType().Name}: {e.InnerException.Message})";
}
