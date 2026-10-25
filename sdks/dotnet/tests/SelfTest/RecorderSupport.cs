using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;

namespace Hairspring;

/// <summary>What the recorder tests share: a recorder over a sink, the basic scenario, threads that start together, sinks that block or throw.</summary>
internal static class RecorderSupport
{
    internal static readonly long OwnPid = Environment.ProcessId;
    internal const string File = "RecorderTests.cs";
    internal static readonly long[] None = [];

    internal static Recorder Make(ISink? sink) => new(new Identity("p1", true, false), sink, 0, OwnPid);

    internal static string W(long n) => n.ToString(System.Globalization.CultureInfo.InvariantCulture).PadLeft(20);

    internal static void Record(Recorder r, Kind kind, bool ok, string id, string msg, params long[] vals) =>
        r.Record(kind, ok, id, msg, vals, File, 1);

    internal static void Basic(Recorder r)
    {
        r.Declare(Kind.Reachable, "basic.reach.never", "never reached", File, 1);
        r.Declare(Kind.Unreachable, "basic.unreach.clean", "never reached either", File, 1);
        r.Declare(Kind.AlwaysOrUnreachable, "basic.aou.never", "never evaluated", File, 1);
        for (var i = 0; i < 5; i++)
        {
            Record(r, Kind.Always, true, "basic.always.ok", "always holds", i, 2);
        }
        Record(r, Kind.Always, false, "basic.always.bad", "always violated", 7, 8);
        for (var i = 0; i < 4; i++)
        {
            Record(r, Kind.Sometimes, i % 2 == 0, "basic.sometimes", "sometimes holds", i);
        }
        Record(r, Kind.Reachable, true, "basic.reach.hit", "is reached");
        r.Work(300);
    }

    internal static string AssertFor(ListSink sink, string id) =>
        sink.OfKind("assert").First(l => l.Contains($"\"id\":\"{id}\"", StringComparison.Ordinal));

    /// <summary>Runs <paramref name="count"/> threads at once, each running <paramref name="body"/> with its index.</summary>
    internal static void InThreads(int count, Action<int> body)
    {
        var failures = new List<Exception>();
        using var go = new ManualResetEventSlim();
        var threads = Enumerable.Range(0, count).Select(i => new Thread(() =>
        {
            try
            {
                go.Wait();
                body(i);
            }
            catch (Exception problem)
            {
                lock (failures)
                {
                    failures.Add(problem);
                }
            }
        })).ToList();
        threads.ForEach(t => t.Start());
        go.Set();
        threads.ForEach(t => t.Join());
        if (failures.Count > 0)
        {
            throw new CheckFailedException("a thread failed: " + failures[0]);
        }
    }

    /// <summary>A sink whose first line blocks until released, so a report can be caught half written.</summary>
    internal sealed class BlockingSink : ISink
    {
        public readonly ListSink Inner = new();
        public readonly ManualResetEventSlim Entered = new();
        public readonly ManualResetEventSlim Release = new();

        public bool WriteLine(byte[] line)
        {
            Entered.Set();
            Release.Wait();
            return Inner.WriteLine(line);
        }

        public void Close()
        {
        }
    }

    internal sealed class ThrowingSink : ISink
    {
        public bool WriteLine(byte[] line) => throw new InvalidOperationException("sink failed");

        public void Close()
        {
        }
    }

    internal static readonly TimeSpan Blocked = TimeSpan.FromSeconds(10);
}
