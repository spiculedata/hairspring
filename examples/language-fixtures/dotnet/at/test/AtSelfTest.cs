using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Threading;

namespace LangAt.SelfTest;

/// <summary>
/// The fixture's own tests: its command line, its line, its barrier, its invariants and its workers. No framework, one
/// process; run with <c>HAIRSPRING_EVENTS=off</c> so the SDK writes no report for the evaluations these tests make. Exits 0
/// when every case passed, 1 otherwise.
/// </summary>
internal static class AtSelfTest
{
    private sealed class CheckFailedException(string message) : Exception(message);

    private static void Equal<T>(T want, T got)
    {
        var same = want is IEnumerable<int> wantInts && got is IEnumerable<int> gotInts
            ? wantInts.SequenceEqual(gotInts)
            : EqualityComparer<T>.Default.Equals(want, got);
        if (!same)
        {
            throw new CheckFailedException($"want {Show(want)}, got {Show(got)}");
        }
    }

    private static void Yes(bool condition, string what)
    {
        if (!condition)
        {
            throw new CheckFailedException($"not true: {what}");
        }
    }

    private static string Show<T>(T value) => value is IEnumerable<int> ints ? "[" + string.Join(", ", ints) + "]" : value?.ToString() ?? "null";

    private static Mode? Parse(params string[] args) => Mode.Parse(args);

    /// <summary>A stdout that records how many write calls it saw and what they held.</summary>
    private sealed class Recording
    {
        public readonly List<byte[]> Writes = new();

        public bool Write(byte[] line)
        {
            Writes.Add(line);
            return true;
        }

        public string Text() => Encoding.ASCII.GetString(Writes.SelectMany(w => w).ToArray());
    }

    private static int MainRun(string[] args, Func<byte[], bool> stdout, StringWriter err, Func<Mode, RaceResult> workload) =>
        Cli.Run(args, stdout, err, workload, () => 0x0123456789abcdefUL);

    private static string Hex16(ulong value)
    {
        const string digits = "0123456789abcdef";
        var text = new StringBuilder();
        for (var shift = 60; shift >= 0; shift -= 4)
        {
            text.Append(digits[(int)((value >> shift) & 0xf)]);
        }
        return text.ToString();
    }

    private static readonly (string Name, Action Body)[] Cases =
    {
        ("mode: buggy, fixed and wide are the three modes", () =>
        {
            Equal(Mode.Buggy, Parse("buggy"));
            Equal(Mode.Fixed, Parse("fixed"));
            Equal(Mode.Wide, Parse("wide"));
        }),
        ("mode: nothing, an unknown mode, a wrong-case mode and an extra argument are usage errors", () =>
        {
            Yes(Parse() is null, "no argument");
            Yes(Parse("racy") is null, "unknown");
            Yes(Parse("Buggy") is null, "wrong case");
            Yes(Parse("") is null, "empty");
            Yes(Parse("buggy", "extra") is null, "extra argument");
            Yes(Parse("buggy", "--switch=1.0") is null, "no second argument exists in .NET");
        }),
        ("contract: the constants", () =>
        {
            Equal(4, Contract.Threads);
            Equal(1000, Contract.Iterations);
            Equal(1000, Contract.Window);
            Equal(10000, Contract.WindowWide);
            Equal(4000, Contract.Expected);
            Equal("dotnet", Contract.Lang);
        }),
        ("mode: wide differs from buggy only in the busy loop length, fixed only in the lock", () =>
        {
            Equal(Contract.Window, Mode.Buggy.WindowRounds());
            Equal(Contract.WindowWide, Mode.Wide.WindowRounds());
            Equal(Contract.Window, Mode.Fixed.WindowRounds());
            Yes(!Mode.Buggy.IsLocked() && !Mode.Wide.IsLocked() && Mode.Fixed.IsLocked(), "only fixed is locked");
            Equal("wide", Mode.Wide.Wire());
        }),
        ("line: the result line has fixed widths and lowercase hex", () =>
            Equal(
                "LANGAT v1 lang=dotnet mode=buggy final=00000000000000003990 expect=00000000000000004000 " +
                "order=00000000000000ab rand=0123456789abcdef\n",
                LangAtLine.Format(Mode.Buggy, 3990, 0xab, 0x0123456789abcdefUL))),
        ("line: the largest random value still has sixteen digits", () =>
            Yes(LangAtLine.Format(Mode.Fixed, 4000, 0, ulong.MaxValue).Contains(" rand=ffffffffffffffff\n", StringComparison.Ordinal), "rand")),
        ("line: the line is formatted the same under every culture", () =>
        {
            var saved = System.Globalization.CultureInfo.CurrentCulture;
            try
            {
                System.Globalization.CultureInfo.CurrentCulture = new System.Globalization.CultureInfo("ar-SA");
                Yes(LangAtLine.Format(Mode.Fixed, 4000, 1, 2).Contains(" final=00000000000000004000 ", StringComparison.Ordinal), "digits");
            }
            finally
            {
                System.Globalization.CultureInfo.CurrentCulture = saved;
            }
        }),
        ("line: the order is the FNV-1a 64 hash of the arrival list written as ASCII digits", () =>
        {
            Equal(0xaf63dc4c8601ec8cUL, OrderHash.Fnv1a64("a"u8));
            Equal(0x85944171f73967e8UL, OrderHash.Fnv1a64("foobar"u8));
            Equal(OrderHash.Fnv1a64("2031"u8), OrderHash.Of([2, 0, 3, 1]));
            Yes(OrderHash.Of([0, 1, 2, 3]) != OrderHash.Of([3, 2, 1, 0]), "order matters");
        }),
        ("barrier: nobody passes until every party has arrived", () =>
        {
            var barrier = new StartBarrier(4);
            var passedEarly = 0;
            var threads = new List<Thread>();
            for (var i = 0; i < 4; i++)
            {
                var thread = new Thread(() =>
                {
                    var released = barrier.Arrive();
                    if (!released || barrier.Started < 4)
                    {
                        Interlocked.Increment(ref passedEarly);
                    }
                });
                threads.Add(thread);
                thread.Start();
                if (i < 3)
                {
                    Thread.Sleep(20);
                    Equal(0, Volatile.Read(ref passedEarly));
                }
            }
            threads.ForEach(t => t.Join());
            Equal(0, passedEarly);
        }),
        ("barrier: an abort wakes the waiters and tells them the start failed", () =>
        {
            var barrier = new StartBarrier(4);
            var released = -1;
            var waiter = new Thread(() => Volatile.Write(ref released, barrier.Arrive() ? 1 : 0));
            waiter.Start();
            Thread.Sleep(20);
            barrier.Abort();
            waiter.Join();
            Equal(0, released);
            Yes(!barrier.Arrive(), "a later arrival fails too");
        }),
        ("race: spin is a pure function of its argument", () =>
        {
            Equal(Race.Spin(1000), Race.Spin(1000));
            Yes(Race.Spin(1000) != Race.Spin(1001), "different rounds, different value");
            Yes(Race.Spin(0) != Race.Spin(1), "the accumulator moves");
        }),
        ("race: spin computes the rotate-multiply sequence", () =>
        {
            foreach (var rounds in new[] { 0, 1, 2, 7, 64, 1000 })
            {
                var acc = 0xcbf29ce484222325UL;
                for (var i = 0; i < rounds; i++)
                {
                    acc = unchecked((System.Numerics.BitOperations.RotateLeft(acc, 7) ^ (ulong)i) * 0x5851f42d4c957f2dUL);
                }
                Equal(acc, Race.Spin(rounds));
            }
        }),
        ("race: the locked mode never loses an update and every worker arrives once", () =>
        {
            for (var run = 0; run < 20; run++)
            {
                var result = Race.Run(Mode.Fixed, 200);
                Equal(4 * 200L, result.FinalValue);
                Equal(4, result.Started);
                Equal(new[] { 0, 1, 2, 3 }, result.Arrival.OrderBy(x => x).ToArray());
            }
        }),
        ("race: the unlocked mode never gains an update", () =>
        {
            for (var run = 0; run < 20; run++)
            {
                var result = Race.Run(Mode.Buggy, 200);
                Yes(result.FinalValue <= 4 * 200L, $"final {result.FinalValue}");
                Yes(result.FinalValue >= 1L, $"some update survives: {result.FinalValue}");
                Equal(4, result.Arrival.Length);
            }
        }),
        ("main: a good run writes the contract line with one write call and exits 0", () =>
        {
            var output = new Recording();
            var err = new StringWriter();
            Equal(0, MainRun(["buggy"], output.Write, err, _ => new RaceResult(3990, 4, [2, 0, 3, 1])));
            Equal(1, output.Writes.Count);
            Equal(
                "LANGAT v1 lang=dotnet mode=buggy final=00000000000000003990 expect=00000000000000004000 order=" +
                Hex16(OrderHash.Of([2, 0, 3, 1])) + " rand=0123456789abcdef\n",
                output.Text());
            Equal("", err.ToString());
        }),
        ("main: a usage error prints a usage line on stderr, nothing on stdout, and exits 64", () =>
        {
            var output = new Recording();
            var err = new StringWriter();
            Equal(64, MainRun(["racy"], output.Write, err, _ => throw new CheckFailedException("the workload must not run")));
            Equal(0, output.Writes.Count);
            Yes(err.ToString().Contains("usage", StringComparison.Ordinal), "usage line");
        }),
        ("main: a worker that failed is an invariant failure, exit 3, nothing on stdout", () =>
        {
            var output = new Recording();
            var err = new StringWriter();
            Equal(3, MainRun(["fixed"], output.Write, err, _ => throw new RaceFailure("a worker thread failed to start or died")));
            Equal(0, output.Writes.Count);
            Yes(err.ToString().Contains("a worker thread failed", StringComparison.Ordinal), "reason");
        }),
        ("main: an arrival list that is not four entries long is an invariant failure", () =>
        {
            var output = new Recording();
            Equal(3, MainRun(["fixed"], output.Write, new StringWriter(), _ => new RaceResult(4000, 4, [0, 1, 2])));
            Equal(0, output.Writes.Count);
        }),
        ("main: a stdout that cannot be written is an invariant failure, exit 3", () =>
        {
            var err = new StringWriter();
            Equal(3, MainRun(["fixed"], _ => false, err, _ => new RaceResult(4000, 4, [0, 1, 2, 3])));
            Yes(err.ToString().Contains("could not be written", StringComparison.Ordinal), "reason");
        }),
    };

    private static int Main()
    {
        var failed = 0;
        foreach (var (name, body) in Cases)
        {
            try
            {
                body();
                Console.WriteLine($"ok   {name}");
            }
            catch (Exception problem)
            {
                failed++;
                Console.WriteLine(problem is CheckFailedException ? $"FAIL {name}: {problem.Message}" : $"FAIL {name}: {problem}");
            }
        }
        Console.WriteLine($"{Cases.Length - failed} passed, {failed} failed");
        return failed == 0 ? 0 : 1;
    }
}
