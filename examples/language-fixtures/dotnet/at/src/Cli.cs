using System;
using System.IO;
using System.Text;
using Hairspring;

namespace LangAt;

/// <summary>
/// The command line of the program: <c>dotnet LangAt.dll buggy|fixed|wide</c>. It runs the workers, evaluates two properties in
/// the main thread after they are joined (<c>dotnet.counter_exact</c>, an <c>Always</c> with the witness values
/// <c>[final, 4000, 4, 1000]</c>, and <c>dotnet.all_started</c>, a <c>Sometimes</c>), and writes one stdout line in one write.
/// Exit status: 0 whatever the property says; 3 if the program itself failed (a thread did not start, the barrier broke, the
/// arrival list is not four entries long, the line could not be written); 64 for a usage error. The verdict is the SDK's
/// report, not the exit code.
/// </summary>
internal static class Cli
{
    public const string Usage = "usage: dotnet LangAt.dll buggy|fixed|wide";

    /// <summary>Runs the program against the given streams and returns the exit status.</summary>
    public static int Run(string[] args, Func<byte[], bool> writeStdout, TextWriter stderr, Func<Mode, RaceResult> workload, Func<ulong> rand)
    {
        var mode = Mode.Parse(args);
        if (mode is null)
        {
            stderr.WriteLine(Usage);
            return Contract.ExitUsage;
        }
        Hs.Start();
        RaceResult result;
        try
        {
            result = workload(mode);
        }
        catch (RaceFailure failed)
        {
            stderr.WriteLine($"invariant failure: {failed.Message}");
            return Contract.ExitInvariant;
        }
        if (result.Arrival.Length != Contract.Threads)
        {
            stderr.WriteLine($"invariant failure: the arrival list does not hold {Contract.Threads} entries");
            return Contract.ExitInvariant;
        }
        Evaluate(result);
        var line = Encoding.ASCII.GetBytes(LangAtLine.Format(mode, result.FinalValue, OrderHash.Of(result.Arrival), rand()));
        if (!writeStdout(line))
        {
            stderr.WriteLine("invariant failure: the stdout line could not be written");
            return Contract.ExitInvariant;
        }
        return Contract.ExitOk;
    }

    private static void Evaluate(RaceResult result)
    {
        Hs.Always(
            result.FinalValue == Contract.Expected,
            Contract.Lang + ".counter_exact",
            "counter equals T*K",
            [result.FinalValue, Contract.Expected, Contract.Threads, Contract.Iterations]);
        Hs.Sometimes(result.Started == Contract.Threads, Contract.Lang + ".all_started", "all workers started", [result.Started]);
    }
}
