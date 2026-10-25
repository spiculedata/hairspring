using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;

namespace Hairspring;

/// <summary>The recorder: counting under contention, consistent snapshots, and the report it writes.</summary>
internal static class RecorderTests
{
    private static readonly long OwnPid = Environment.ProcessId;
    private const string File = "RecorderTests.cs";
    private static readonly long[] None = [];

    private static Recorder Make(ISink? sink) => new(new Identity("p1", true, false), sink, 0, OwnPid);

    private static string W(long n) => n.ToString(System.Globalization.CultureInfo.InvariantCulture).PadLeft(20);

    private static void Record(Recorder r, Kind kind, bool ok, string id, string msg, params long[] vals) =>
        r.Record(kind, ok, id, msg, vals, File, 1);

    private static void Basic(Recorder r)
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

    private static string AssertFor(ListSink sink, string id) =>
        sink.OfKind("assert").First(l => l.Contains($"\"id\":\"{id}\"", StringComparison.Ordinal));

    /// <summary>Runs <paramref name="count"/> threads at once, each running <paramref name="body"/> with its index.</summary>
    private static void InThreads(int count, Action<int> body)
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
    private sealed class BlockingSink : ISink
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

    private static void ShapeCases(Suite s)
    {
        s.Add("recorder: the report has hello, the catalog, the asserts, work, thread lines and end, in that order", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            Basic(r);
            Check.Yes(r.Report(EndReason.Exit, null), "written");
            var want = new List<string> { "hello" };
            want.AddRange(Enumerable.Repeat("catalog", 7));
            want.AddRange(Enumerable.Repeat("assert", 7));
            want.AddRange(["work", "thread", "end"]);
            Check.Equal(want, sink.Kinds());
            var end = sink.OfKind("end")[0];
            Check.Yes(end.Contains("\"sites\":7,\"evals\":" + W(11) + ",\"dropped\":" + W(0) + ",\"threads\":1}", StringComparison.Ordinal), end);
            Check.Yes(sink.OfKind("work")[0].Contains("\"ops\":" + W(300), StringComparison.Ordinal), "work");
        });
        s.Add("recorder: a witness carries the thread's key, its own ordinal and the values", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            Basic(r);
            r.Report(EndReason.Exit, null);
            var bad = AssertFor(sink, "basic.always.bad");
            Check.Yes(bad.Contains("\"first_false\":{\"thread\":\"t#1\",\"ord\":" + W(6) + ",\"vals\":[7,8,0,0]}", StringComparison.Ordinal), bad);
            var ok = AssertFor(sink, "basic.always.ok");
            Check.Yes(ok.Contains("\"first_true\":{\"thread\":\"t#1\",\"ord\":" + W(1) + ",\"vals\":[0,2,0,0]}", StringComparison.Ordinal), ok);
        });
        s.Add("recorder: the same id from two places is one site with summed counts", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            Record(r, Kind.Sometimes, true, "merge.shared", "one id, two sites", 1);
            Record(r, Kind.Sometimes, false, "merge.shared", "one id, two sites", 1);
            Record(r, Kind.Sometimes, true, "merge.shared", "one id, two sites", 2);
            Record(r, Kind.Sometimes, true, "merge.shared", "one id, two sites", 2);
            r.Report(EndReason.Exit, null);
            Check.Equal(1, sink.OfKind("catalog").Count);
            Check.Equal(1, sink.OfKind("assert").Count);
            Check.Yes(sink.OfKind("assert")[0].Contains("\"hits\":" + W(4) + ",\"trues\":" + W(3) + ",\"falses\":" + W(1), StringComparison.Ordinal), "counts");
            Check.Yes(sink.OfKind("end")[0].Contains("\"sites\":1,", StringComparison.Ordinal), "one site");
        });
        s.Add("recorder: an id used with two kinds is a catalog conflict the engine reports", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            Record(r, Kind.Always, true, "conflict.id", "declared as always");
            Record(r, Kind.Sometimes, true, "conflict.id", "declared as sometimes");
            r.Report(EndReason.Exit, null);
            Check.Equal(2, sink.OfKind("catalog").Count);
            Check.Equal(1, sink.OfKind("assert").Count);
            Check.Yes(sink.OfKind("assert")[0].Contains("\"hits\":" + W(1), StringComparison.Ordinal), "the first kind's counts");
            Check.Yes(sink.OfKind("end")[0].Contains("\"sites\":1,", StringComparison.Ordinal), "one distinct id");
        });
        s.Add("recorder: a declared site that never runs is reported with zero hits", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            r.Declare(Kind.Reachable, "wal.reset.reached", "the WAL was reset", File, 1);
            r.Report(EndReason.Exit, null);
            var line = AssertFor(sink, "wal.reset.reached");
            Check.Yes(line.Contains("\"hits\":" + W(0) + ",\"trues\":" + W(0) + ",\"falses\":" + W(0), StringComparison.Ordinal), line);
            Check.Yes(line.Contains("\"first_true\":null", StringComparison.Ordinal), line);
        });
        s.Add("recorder: evaluating a declared site is the same site", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            r.Declare(Kind.Reachable, "x", "m", File, 1);
            Record(r, Kind.Reachable, true, "x", "m");
            r.Report(EndReason.Exit, null);
            Check.Equal(1, sink.OfKind("catalog").Count);
            Check.Yes(AssertFor(sink, "x").Contains("\"hits\":" + W(1), StringComparison.Ordinal), "one hit");
        });
        s.Add("recorder: the caller's file and line are in the catalog line", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            r.Record(Kind.Reachable, true, "where", "m", None, "src/Where.cs", 42);
            r.Report(EndReason.Exit, null);
            Check.Yes(sink.OfKind("catalog")[0].Contains("\"loc\":{\"file\":\"src/Where.cs\",\"line\":42}", StringComparison.Ordinal), sink.OfKind("catalog")[0]);
        });
        s.Add("where: only the last two parts of a path are kept", () =>
        {
            Check.Equal("src/Race.cs", Where.Short("/_/examples/language-fixtures/dotnet/at/src/Race.cs"));
            Check.Equal("src/Race.cs", Where.Short("C:\\build\\at\\src\\Race.cs"));
            Check.Equal("Race.cs", Where.Short("Race.cs"));
            Check.Equal("<unknown>", Where.Short(""));
            Check.Equal("<unknown>", Where.Short(null));
        });
    }

    private static void ValidationCases(Suite s)
    {
        s.Add("recorder: a bad id or message or a fifth value is refused and counts nothing", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            Check.Raises<ArgumentException>(() => Record(r, Kind.Always, true, "", "m"));
            Check.Raises<ArgumentException>(() => Record(r, Kind.Always, true, "x", new string('m', 513)));
            Check.Raises<ArgumentException>(() => Record(r, Kind.Always, true, "x", "m", 1, 2, 3, 4, 5));
            Check.Raises<ArgumentException>(() => r.Declare(Kind.Always, "a\0b", "", File, 1));
            Check.Raises<ArgumentOutOfRangeException>(() => r.Work(-1));
            Check.Raises<ArgumentOutOfRangeException>(() => Record(r, (Kind)9, true, "x", "m"));
            r.Report(EndReason.Exit, null);
            Check.Equal(0, sink.OfKind("catalog").Count);
            Check.Yes(sink.OfKind("end")[0].Contains("\"evals\":" + W(0), StringComparison.Ordinal), "nothing counted");
        });
        s.Add("recorder: a bad message is refused on every call, also for a site that is already known", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            Record(r, Kind.Always, true, "x", "fine");
            Check.Raises<ArgumentException>(() => Record(r, Kind.Always, true, "x", new string('m', 513)));
            Check.Raises<ArgumentException>(() => r.Declare(Kind.Always, "x", new string('m', 513), File, 1));
            Check.Raises<ArgumentNullException>(() => r.Record(Kind.Always, true, "x", null!, None, File, 1));
            Record(r, Kind.Always, true, "x", new string('\u20ac', 170)); // 510 bytes: fits
            Check.Raises<ArgumentException>(() => Record(r, Kind.Always, true, "x", new string('\u20ac', 171)));
            r.Report(EndReason.Exit, null);
            Check.Yes(sink.OfKind("end")[0].Contains("\"evals\":" + W(2), StringComparison.Ordinal), "only the two good calls counted");
        });
        s.Add("recorder: an id with a NUL is refused on every spelling", () =>
        {
            var r = Make(new ListSink());
            Check.Raises<ArgumentException>(() => Record(r, Kind.Always, true, "a\0b", "m"));
            Check.Raises<ArgumentException>(() => Record(r, Kind.Always, true, "a\0b", "m"));
        });
        s.Add("recorder: ids and messages at their limits are accepted", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            Record(r, Kind.Reachable, true, new string('x', 128), new string('m', 512));
            Record(r, Kind.Reachable, true, "bad id$with:odd/chars", "quote \" and backslash \\ in the message");
            r.Report(EndReason.Exit, null);
            Check.Yes(sink.OfKind("catalog")[0].Contains("\"id\":\"" + new string('x', 128) + "\"", StringComparison.Ordinal), "long id");
            Check.Yes(sink.OfKind("catalog")[1].Contains("\"id\":\"bad_id_with:odd/chars\"", StringComparison.Ordinal), "sanitised id");
        });
        s.Add("recorder: control and bidirectional characters in a message are sent as question marks", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            Record(r, Kind.Reachable, true, "ctl", "a\u202eb\u0007c\u2028d");
            r.Report(EndReason.Exit, null);
            var line = sink.OfKind("catalog")[0];
            Check.Yes(line.Contains("\"msg\":\"a???b?c???d\"", StringComparison.Ordinal), line);
        });
        s.Add("recorder: more distinct ids than the catalog holds are dropped and counted, and end.sites stays within what a parser accepts", () =>
        {
            var sink = new ListSink();
            var r = Make(sink);
            for (var i = 0; i < SiteTable.MaxDistinct + 5; i++)
            {
                r.Declare(Kind.Reachable, "id." + i, "", File, 1);
            }
            r.Record(Kind.Reachable, true, "id.0", "", None, File, 1);
            r.Record(Kind.Reachable, true, "id." + (SiteTable.MaxDistinct + 1), "", None, File, 1);
            r.Report(EndReason.Exit, null);
            Check.Equal(SiteTable.MaxDistinct, sink.OfKind("catalog").Count);
            var end = sink.OfKind("end")[0];
            Check.Yes(end.Contains("\"sites\":4096,", StringComparison.Ordinal), end);
            Check.Yes(end.Contains("\"dropped\":" + W(5 + 1), StringComparison.Ordinal), end);
        });
    }

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

    private sealed class ThrowingSink : ISink
    {
        public bool WriteLine(byte[] line) => throw new InvalidOperationException("sink failed");

        public void Close()
        {
        }
    }

    private static void IdentityCases(Suite s)
    {
        s.Add("recorder: a child process is given the tag and the owning pid", () =>
        {
            var r = Make(new ListSink());
            Check.Equal("p1", r.ChildEnvironment()["HAIRSPRING_PROC"]);
            Check.Equal(OwnPid.ToString(System.Globalization.CultureInfo.InvariantCulture), r.ChildEnvironment()["HAIRSPRING_PROC_PID"]);
            r.Report(EndReason.Exit, null);
            Check.Equal(OwnPid + ":a", r.ChildEnvironment()["HAIRSPRING_PROC_PID"]);
        });
        s.Add("recorder: an unstable root tag is passed on as unstable", () =>
        {
            var r = new Recorder(new Identity("p1.s9", false, false), new ListSink(), 0, OwnPid);
            Check.Equal(OwnPid + ":u", r.ChildEnvironment()["HAIRSPRING_PROC_PID"]);
        });
        s.Add("recorder: a refused harness tag is counted and the default tag is used", () =>
        {
            var sink = new ListSink();
            var r = Recorder.FromEnvironment(n => n == "HAIRSPRING_PROC" ? "bad tag" : null, sink, 0, OwnPid);
            r.Report(EndReason.Exit, null);
            Check.Yes(sink.OfKind("hello")[0].Contains("\"proc\":\"p1\"", StringComparison.Ordinal), "default tag");
            Check.Yes(sink.OfKind("end")[0].Contains("\"dropped\":" + W(1), StringComparison.Ordinal), "counted");
        });
        s.Add("recorder: a spawned child takes the parent's tag with its own pid and says it is unstable", () =>
        {
            var sink = new ListSink();
            var parent = OwnPid + 7;
            var r = Recorder.FromEnvironment(
                n => n switch { "HAIRSPRING_PROC" => "p1", "HAIRSPRING_PROC_PID" => parent.ToString(System.Globalization.CultureInfo.InvariantCulture), _ => null },
                sink, 0, OwnPid);
            r.Report(EndReason.Exit, null);
            var hello = sink.OfKind("hello")[0];
            Check.Yes(hello.Contains($"\"proc\":\"p1.s{OwnPid}\"", StringComparison.Ordinal), hello);
            Check.Yes(hello.EndsWith("\"tag_stable\":false}\n", StringComparison.Ordinal), hello);
        });
    }

    private static readonly TimeSpan Blocked = TimeSpan.FromSeconds(10);

    public static void Register(Suite s)
    {
        ShapeCases(s);
        ValidationCases(s);
        ThreadingCases(s);
        ReportCases(s);
        IdentityCases(s);
    }
}
