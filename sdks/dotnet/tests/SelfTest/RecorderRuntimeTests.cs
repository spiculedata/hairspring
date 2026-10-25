using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using static Hairspring.RecorderSupport;

namespace Hairspring;

/// <summary>The recorder at run time: counting under contention, consistent snapshots, and writing the report once without holding a lock.</summary>
internal static class RecorderRuntimeTests
{
    private static void ThreadingCases(Suite s)
    {
        s.Add("recorder: eight contending threads leave exact totals", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            const int each = 20_000;
            InThreads(8, _ =>
            {
                for (var i = 0; i < each; i++)
                {
                    r.Record(Kind.Sometimes, i % 2 == 0, "contended", "m", [i], File, 1);
                }
            });
            r.Report(EndReason.Exit, null);
            var line = AssertFor(sink, "contended");
            Check.Yes(line.Contains("\"hits\":" + W(8L * each) + ",\"trues\":" + W(4L * each) + ",\"falses\":" + W(4L * each), StringComparison.Ordinal), line);
            Check.Equal(8, sink.OfKind("thread").Count);
            var evals = sink.OfKind("thread").Sum(l =>
            {
                var from = l.IndexOf("\"evals\":", StringComparison.Ordinal) + 8;
                return long.Parse(l[from..l.IndexOf(",\"tape", StringComparison.Ordinal)].Trim(), System.Globalization.CultureInfo.InvariantCulture);
            });
            Check.Equal(8L * each, evals);
        });
        s.Add("recorder: a snapshot taken while threads evaluate is consistent", () =>
        {
            var r = Make(new ListSink());
            var stop = false;
            var threads = Enumerable.Range(0, 4).Select(_ => new Thread(() =>
            {
                long i = 0;
                while (!Volatile.Read(ref stop))
                {
                    r.Record(Kind.Always, i % 3 != 0, "live", "m", [i], File, 1);
                    i++;
                }
            })).ToList();
            threads.ForEach(t => t.Start());
            try
            {
                for (var n = 0; n < 3000; n++)
                {
                    var snap = r.Snapshot();
                    ulong hits = 0;
                    foreach (var row in snap.Asserts)
                    {
                        hits += row.Trues + row.Falses;
                        Check.Yes((row.FirstTrue is not null) == (row.Trues > 0), "a true witness appears exactly when trues is non-zero");
                        Check.Yes(row.Falses > 0 || row.FirstFalse is null, "no false witness behind a zero count");
                    }
                    Check.Equal("the evaluations of the thread lines equal the hits of the assert lines", snap.Evals, hits);
                }
            }
            finally
            {
                Volatile.Write(ref stop, true);
                threads.ForEach(t => t.Join());
            }
        });
        s.Add("recorder: threads beyond the table are capped and counted without a failure", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            for (var i = 0; i < 300; i++)
            {
                var t = new Thread(() => r.Record(Kind.Sometimes, true, "threads.each", "m", None, File, 1));
                t.Start();
                t.Join();
            }
            r.Report(EndReason.Exit, null);
            Check.Equal(256, sink.OfKind("thread").Count);
            var end = sink.OfKind("end")[0];
            Check.Yes(end.Contains("\"threads\":256}", StringComparison.Ordinal), end);
            Check.No(end.Contains("\"dropped\":" + W(0), StringComparison.Ordinal), "dropped");
        });
        s.Add("recorder: a thread-pool thread takes a slot of its own like any other", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            Record(r, Kind.Reachable, true, "main", "m");
            System.Threading.Tasks.Task.Run(() => Record(r, Kind.Reachable, true, "pool", "m")).Wait();
            r.Report(EndReason.Exit, null);
            Check.Equal(2, sink.OfKind("thread").Count);
        });
        s.Add("recorder: NameThread gives the calling thread a stable key", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            Check.Yes(r.NameThread("worker", 3), "claimed");
            Record(r, Kind.Reachable, true, "x", "m");
            r.Report(EndReason.Exit, null);
            var line = sink.OfKind("thread")[0];
            Check.Yes(line.Contains("\"thread\":\"worker-3\"", StringComparison.Ordinal), line);
            Check.Yes(line.EndsWith(",\"key_stable\":true }\n", StringComparison.Ordinal), line);
            Check.Yes(AssertFor(sink, "x").Contains("\"thread\":\"worker-3\"", StringComparison.Ordinal), "the witness names it too");
        });
        s.Add("recorder: a refused NameThread call takes no slot", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            var refused = new Thread(() =>
            {
                Check.Raises<ArgumentException>(() => r.NameThread("", 0));
                Check.Raises<ArgumentOutOfRangeException>(() => r.NameThread("a", -1));
            });
            refused.Start();
            refused.Join(); // a thread that did nothing but make refused calls
            Record(r, Kind.Reachable, true, "x", "m");
            r.Report(EndReason.Exit, null);
            Check.Yes(AssertFor(sink, "x").Contains("\"thread\":\"t#1\"", StringComparison.Ordinal), AssertFor(sink, "x"));
            Check.Equal(1, sink.OfKind("thread").Count);
        });
        s.Add("recorder: work counts per thread and is reported as one total", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            InThreads(4, _ =>
            {
                for (var i = 0; i < 1000; i++)
                {
                    r.Work(1);
                }
            });
            r.Report(EndReason.Exit, null);
            Check.Yes(sink.OfKind("work")[0].Contains("\"ops\":" + W(4000), StringComparison.Ordinal), sink.OfKind("work")[0]);
        });
        s.Add("recorder: work saturates instead of wrapping", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            r.Work(long.MaxValue);
            r.Work(long.MaxValue);
            r.Work(long.MaxValue);
            r.Report(EndReason.Exit, null);
            Check.Yes(sink.OfKind("work")[0].Contains("18446744073709551615", StringComparison.Ordinal), sink.OfKind("work")[0]);
        });
    }

    private static void ReportCases(Suite s)
    {
        s.Add("recorder: no lock is held while a line is being written", () =>
        {
            var sink = new BlockingSink();
            var r = Make(sink);
            Record(r, Kind.Reachable, true, "x", "m");
            var writer = new Thread(() => r.Report(EndReason.Explicit, null));
            writer.Start();
            Check.Yes(sink.Entered.Wait(Blocked), "the writer started");
            // The writer is inside the sink. Other threads must still be able to record, snapshot and name themselves.
            var others = new Thread(() =>
            {
                Record(r, Kind.Reachable, true, "y", "m");
                r.Work(1);
                r.Snapshot();
                r.ChildEnvironment();
            });
            others.Start();
            Check.Yes(others.Join(Blocked), "a thread was stuck behind the writer");
            sink.Release.Set();
            writer.Join();
        });
        s.Add("recorder: the exit hook waits for a report another thread is still writing", () =>
        {
            var sink = new BlockingSink();
            var r = Make(sink);
            Record(r, Kind.Reachable, true, "x", "m");
            var explicitReport = new Thread(() => r.Report(EndReason.Explicit, null));
            explicitReport.Start();
            Check.Yes(sink.Entered.Wait(Blocked), "the writer started");
            var hookReturned = false;
            var hook = new Thread(() =>
            {
                r.Report(EndReason.Exit, null);
                Volatile.Write(ref hookReturned, true);
            });
            hook.Start();
            Thread.Sleep(300);
            Check.No(Volatile.Read(ref hookReturned), "the hook returned while the report was half written");
            sink.Release.Set();
            Check.Yes(hook.Join(TimeSpan.FromSeconds(2)), "the hook returned promptly once the report was done"); // woken by the writer, not by the 5 second bound
            explicitReport.Join();
            Check.Equal(1, sink.Inner.OfKind("end").Count);
        });
        s.Add("recorder: a crash report also waits for a report another thread is still writing", () =>
        {
            var sink = new BlockingSink();
            var r = Make(sink);
            Record(r, Kind.Reachable, true, "x", "m");
            var explicitReport = new Thread(() => r.Report(EndReason.Explicit, null));
            explicitReport.Start();
            Check.Yes(sink.Entered.Wait(Blocked), "the writer started");
            var returned = false;
            var crash = new Thread(() =>
            {
                r.Report(EndReason.Signal, 6);
                Volatile.Write(ref returned, true);
            });
            crash.Start();
            Thread.Sleep(300);
            Check.No(Volatile.Read(ref returned), "the crash report returned while the report was half written");
            sink.Release.Set();
            Check.Yes(crash.Join(TimeSpan.FromSeconds(2)), "returned once the report was done");
            explicitReport.Join();
        });
        s.Add("recorder: the report is written once and later evaluations are not reported", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            Record(r, Kind.Reachable, true, "explicit.before", "m");
            Check.Yes(r.Report(EndReason.Explicit, null), "first");
            var written = sink.Lines().Count;
            Record(r, Kind.Reachable, true, "explicit.after", "m");
            Check.No(r.Report(EndReason.Exit, null), "second");
            Check.Equal(written, sink.Lines().Count);
            Check.Yes(sink.OfKind("end")[0].Contains("\"reason\":\"explicit\"", StringComparison.Ordinal), "reason");
        });
        s.Add("recorder: a process that did not start the SDK writes nothing", () =>
        {
            var sink = new ListSink();
            var r = new Recorder(new Identity("p1", true, false), sink, 0, OwnPid, () => OwnPid + 1);
            Record(r, Kind.Reachable, true, "x", "m");
            Check.No(r.Report(EndReason.Exit, null), "report");
            Check.Equal(0, sink.Lines().Count);
        });
        s.Add("recorder: a forked child never reports even if the parent has not", () =>
        {
            var sink = new ListSink();
            var pid = OwnPid;
            var r = new Recorder(new Identity("p1", true, false), sink, 0, OwnPid, () => Interlocked.Read(ref pid));
            Record(r, Kind.Reachable, true, "x", "m");
            Interlocked.Exchange(ref pid, OwnPid + 5); // what getpid says in the child after a fork
            Check.No(r.Report(EndReason.Exit, null), "the child reported");
            Check.Equal(0, sink.Lines().Count);
            Check.Equal(OwnPid + ":a", r.ChildEnvironment()["HAIRSPRING_PROC_PID"]);
        });
        s.Add("recorder: with the output off the program counts as usual and writes nothing", () =>
        {
            var r = Make(null);
            Record(r, Kind.Reachable, true, "x", "m");
            Check.No(r.Report(EndReason.Exit, null), "report");
        });
        s.Add("recorder: a line the sink refuses is counted in end.dropped, and the end line is still sent", () =>
        {
            var sink = new ListSink(line => line.Contains("\"kind\":\"work\"", StringComparison.Ordinal));
            var r = Make(sink);
            r.Work(1);
            r.Report(EndReason.Exit, null);
            Check.Yes(sink.OfKind("end")[0].Contains("\"dropped\":" + W(1), StringComparison.Ordinal), sink.OfKind("end")[0]);
        });
        s.Add("recorder: a sink that throws costs the line, not the program", () =>
        {
            var r = Make(new ThrowingSink());
            Record(r, Kind.Reachable, true, "x", "m");
            Check.Yes(r.Report(EndReason.Exit, null), "reported");
        });
        s.Add("recorder: a failed report is announced through the sink, not around it", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            r.WarnAboutFailure("the report could not be written", new InvalidOperationException("boom"));
            Check.Equal(new[] { "hairspring: the report could not be written: InvalidOperationException: boom\n" }, sink.Lines());
            Make(null).WarnAboutFailure("x", new InvalidOperationException("y")); // output off: nothing to write to, nothing thrown
        });
        s.Add("recorder: the status the program recorded is in the end line", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            r.SetStatus(3);
            r.Report(EndReason.Exit, null);
            Check.Yes(sink.OfKind("end")[0].Contains("\"status\":" + W(3), StringComparison.Ordinal), "status");
        });
        s.Add("recorder: an override replaces the recorded status", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            r.SetStatus(3);
            r.Report(EndReason.Signal, 6);
            Check.Yes(sink.OfKind("end")[0].Contains("\"reason\":\"signal\",\"status\":" + W(6), StringComparison.Ordinal), sink.OfKind("end")[0]);
        });
    }

    public static void Register(Suite s)
    {
        ThreadingCases(s);
        ReportCases(s);
    }
}
