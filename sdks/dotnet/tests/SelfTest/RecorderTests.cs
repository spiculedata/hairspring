using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using static Hairspring.RecorderSupport;

namespace Hairspring;

/// <summary>The recorder: what the report says, what a call refuses, and the tag a child gets.</summary>
internal static class RecorderTests
{
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

    public static void Register(Suite s)
    {
        ShapeCases(s);
        ValidationCases(s);
        IdentityCases(s);
    }
}
