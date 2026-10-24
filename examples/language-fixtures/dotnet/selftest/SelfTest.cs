using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;

namespace LangAt.SelfTest;

/// <summary>Unit checks of the probe, with no test framework: each case is a name and an action that throws on failure.</summary>
internal static class SelfTest
{
    private const string UsageStart = "usage: ";

    private static readonly (string Name, Action Body)[] Cases =
    {
        ("fnv1a64 matches the published vectors", () =>
        {
            Equal(0xcbf29ce484222325UL, Line.Fnv1a64(Encoding.ASCII.GetBytes("")));
            Equal(0xaf63dc4c8601ec8cUL, Line.Fnv1a64(Encoding.ASCII.GetBytes("a")));
            Equal(0x85944171f73967e8UL, Line.Fnv1a64(Encoding.ASCII.GetBytes("foobar")));
        }),
        ("a thread order hashes as bare ASCII digits", () =>
        {
            Equal(Line.Fnv1a64(Encoding.ASCII.GetBytes("2013")), Line.HashDigits(new[] { 2, 0, 1, 3 }));
            NotEqual(Line.HashDigits(new[] { 0, 1, 2, 3 }), Line.HashDigits(new[] { 3, 2, 1, 0 }));
        }),
        ("the line has the six-field contract form", () =>
        {
            var fields = new Fields(0xab, 0xcd, 0x1234, 5_000_000, ulong.MaxValue);
            Equal(
                "PROBE v1 lang=dotnet lock_order=00000000000000ab arrival=00000000000000cd " +
                "rand=0000000000001234 vtime_ns=00000000000005000000 extra=ffffffffffffffff\n",
                Line.Format(fields));
        }),
        ("the largest virtual time still has twenty digits", () =>
        {
            True(Line.Format(new Fields(0, 0, 0, ulong.MaxValue, 0)).Contains(" vtime_ns=18446744073709551615 "));
        }),
        ("the same fields always give the same line", () =>
        {
            var fields = new Fields(1, 2, 3, 4, 5);
            Equal(Line.Format(fields), Line.Format(fields));
        }),
        ("the line is formatted the same under every culture", () =>
        {
            var saved = System.Globalization.CultureInfo.CurrentCulture;
            try
            {
                System.Globalization.CultureInfo.CurrentCulture = new System.Globalization.CultureInfo("ar-SA");
                True(Line.Format(new Fields(1, 2, 3, 4, 5)).Contains(" vtime_ns=00000000000000000004 "));
            }
            finally
            {
                System.Globalization.CultureInfo.CurrentCulture = saved;
            }
        }),
        ("a clock difference is converted with the tick frequency", () =>
        {
            Equal(5UL, Clock.NanosBetween(10, 15, 1_000_000_000));
            Equal(500UL, Clock.NanosBetween(10, 15, 10_000_000));
            Equal(5_000UL, Clock.NanosBetween(10, 15, 1_000_000));
            Equal(0UL, Clock.NanosBetween(15, 10, 1_000_000_000));
        }),
        ("the clock difference of a long run does not overflow: it saturates", () =>
        {
            Equal(3_000_000_000_000UL, Clock.NanosBetween(0, 3_000_000_000_000, 1_000_000_000));
            Equal(ulong.MaxValue, Clock.NanosBetween(0, long.MaxValue, 1));
        }),
        ("the busy loop is deterministic and depends on its count", () =>
        {
            Equal(Workload.Spin(100), Workload.Spin(100));
            NotEqual(Workload.Spin(100), Workload.Spin(101));
            NotEqual(Workload.Spin(0), Workload.Spin(1));
        }),
        ("a run takes the lock and finishes once per thread", () =>
        {
            var outcome = Workload.Run();
            SequenceEqual(new[] { 0, 1, 2, 3 }, outcome.LockOrder.OrderBy(i => i));
            SequenceEqual(new[] { 0, 1, 2, 3 }, outcome.Arrival.OrderBy(i => i));
            SequenceEqual(new[] { 0, 1, 2, 3 }, outcome.CompletionOrder.OrderBy(i => i));
        }),
        ("the unlocked counter ends between one and every round", () =>
        {
            var outcome = Workload.Run();
            True(outcome.RacyFinal >= 1 && outcome.RacyFinal <= Workload.Threads * Workload.Rounds, outcome.RacyFinal.ToString());
        }),
        ("a run reports a nonzero elapsed time", () => True(Workload.Run().ElapsedNs > 0)),
        ("the fields hash each order and pass the rest through", () =>
        {
            var outcome = new Outcome(new[] { 0, 1, 2, 3 }, new[] { 3, 2, 1, 0 }, 800, new[] { 1, 0, 3, 2 }, 7, 42);
            var fields = Cli.FieldsOf(outcome);
            Equal(Line.HashDigits(new[] { 0, 1, 2, 3 }), fields.LockOrder);
            Equal(Line.HashDigits(new[] { 3, 2, 1, 0 }), fields.Arrival);
            Equal(Line.HashDigits(new[] { 1, 0, 3, 2 }), fields.Extra);
            Equal((7UL, 42UL), (fields.Rand, fields.VtimeNs));
        }),
        ("a run with no argument writes exactly one whole line with one write and returns 0", () =>
        {
            var stdout = new CountingStream();
            var stderr = new StringWriter();
            Equal(0, Cli.Run(Array.Empty<string>(), stdout, stderr));
            Equal(1, stdout.Writes);
            Equal(string.Empty, stderr.ToString());
            var text = Encoding.ASCII.GetString(stdout.Written.ToArray());
            True(text.StartsWith("PROBE v1 lang=dotnet ", StringComparison.Ordinal), text);
            True(text.EndsWith('\n'), text);
            Equal(1, text.Count(c => c == '\n'));
        }),
        ("any argument is a usage error: exit 64, a usage line on stderr, nothing on stdout", () =>
        {
            foreach (var arg in new[] { "x", "--switch=1.0", "" })
            {
                var stdout = new CountingStream();
                var stderr = new StringWriter();
                Equal(64, Cli.Run(new[] { arg }, stdout, stderr));
                Equal(0, stdout.Writes);
                True(stderr.ToString().StartsWith(UsageStart, StringComparison.Ordinal), stderr.ToString());
            }
        }),
        ("an unwritable stdout is an internal failure: exit 3 and a message on stderr", () =>
        {
            var stderr = new StringWriter();
            Equal(3, Cli.Run(Array.Empty<string>(), new BrokenStream(), stderr));
            True(stderr.ToString().Contains("internal invariant failed", StringComparison.Ordinal), stderr.ToString());
        }),
        ("a worker that did not finish is an internal failure", () =>
        {
            var failure = Throws<InvariantException>(() => Workload.Check(new[] { 0, 1, 2 }, new[] { 0, 1, 2, 3 }, new[] { 0, 1, 2, 3 }));
            True(failure.Message.Length > 0);
            Throws<InvariantException>(() => Workload.Check(new[] { 0, 1, 2, 3 }, new[] { 0, 1, 2 }, new[] { 0, 1, 2, 3 }));
            Throws<InvariantException>(() => Workload.Check(new[] { 0, 1, 2, 3 }, new[] { 0, 1, 2, 3 }, new[] { 0, 1, 2 }));
            Workload.Check(new[] { 0, 1, 2, 3 }, new[] { 0, 1, 2, 3 }, new[] { 0, 1, 2, 3 });
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
            catch (Exception e)
            {
                failed++;
                Console.WriteLine($"FAIL {name}: {e.Message}");
            }
        }
        Console.WriteLine(failed == 0 ? $"selftest ok: {Cases.Length} cases" : $"selftest FAILED: {failed} of {Cases.Length} cases");
        return failed == 0 ? 0 : 1;
    }

    private static void True(bool condition, string? detail = null)
    {
        if (!condition) throw new InvalidOperationException(detail ?? "condition is false");
    }

    private static void Equal<T>(T expected, T actual)
    {
        if (!EqualityComparer<T>.Default.Equals(expected, actual))
            throw new InvalidOperationException($"expected {expected}, got {actual}");
    }

    private static void NotEqual<T>(T first, T second)
    {
        if (EqualityComparer<T>.Default.Equals(first, second)) throw new InvalidOperationException($"both are {first}");
    }

    private static void SequenceEqual(IEnumerable<int> expected, IEnumerable<int> actual)
    {
        if (!expected.SequenceEqual(actual))
            throw new InvalidOperationException($"expected [{string.Join(",", expected)}], got [{string.Join(",", actual)}]");
    }

    private static TException Throws<TException>(Action body) where TException : Exception
    {
        try
        {
            body();
        }
        catch (TException e)
        {
            return e;
        }
        throw new InvalidOperationException($"{typeof(TException).Name} was not thrown");
    }

    /// <summary>Records every call to Write, so the one-write rule can be checked.</summary>
    private sealed class CountingStream : MemoryStream
    {
        public int Writes { get; private set; }
        public List<byte> Written { get; } = new();

        public override void Write(byte[] buffer, int offset, int count)
        {
            Writes++;
            Written.AddRange(buffer.Skip(offset).Take(count));
        }

        public override void Write(ReadOnlySpan<byte> buffer)
        {
            Writes++;
            Written.AddRange(buffer.ToArray());
        }
    }

    private sealed class BrokenStream : MemoryStream
    {
        public override void Write(byte[] buffer, int offset, int count) => throw new IOException("no space left on device");
        public override void Write(ReadOnlySpan<byte> buffer) => throw new IOException("no space left on device");
    }
}
