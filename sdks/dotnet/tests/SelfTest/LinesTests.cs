using System.Text;

namespace Hairspring;

/// <summary>One function per protocol line kind: exact text, fixed widths, no witness behind a zero count.</summary>
internal static class LinesTests
{
    private static Rows.Proc Proc() => Proc("p1", true, 7, 0);

    private static Rows.Proc Proc(string tag, bool stable, int ids, ulong dropped) =>
        new(tag, stable, false, 0xFB6CF21A75AAD6EFUL, ids, dropped);

    private static string W(long n) => n.ToString(System.Globalization.CultureInfo.InvariantCulture).PadLeft(20);

    private static string Wit(string text) => text.PadRight(Fmt.WitnessWidth);

    private static string Text(byte[] line) => Encoding.ASCII.GetString(line);

    private static Rows.Witness Witness(string key, ulong ord, params long[] vals) => new(key, ord, vals);

    public static void Register(Suite s)
    {
        s.Add("lines: hello names the sdk and the catalog hash", () =>
            Check.Equal(
                "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"hello\",\"sdk\":\"dotnet\",\"sdk_version\":\"" + SdkInfo.Version +
                "\",\"proc\":\"p1\",\"catalog_hash\":\"fb6cf21a75aad6ef\",\"signal_handlers\":false}\n",
                Text(Lines.Hello(Proc()))));
        s.Add("lines: hello flags an unstable tag", () =>
        {
            var line = Text(Lines.Hello(Proc("p1", false, 7, 0)));
            Check.Yes(line.EndsWith("\"signal_handlers\":false,\"tag_stable\":false}\n", System.StringComparison.Ordinal), line);
        });
        s.Add("lines: catalog carries text and location", () =>
        {
            var row = new Rows.CatalogRow("basic.always.ok", Kind.Always, "always holds", "Sample.cs", 31);
            Check.Equal(
                "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"catalog\",\"proc\":\"p1\",\"id\":\"basic.always.ok\"," +
                "\"type\":\"always\",\"must_hit\":true,\"msg\":\"always holds\"," +
                "\"loc\":{\"file\":\"Sample.cs\",\"line\":31}}\n",
                Text(Lines.Catalog(Proc(), row)));
        });
        s.Add("lines: must_hit is false for unreachable and always_or_unreachable", () =>
        {
            foreach (var kind in new[] { Kind.Unreachable, Kind.AlwaysOrUnreachable })
            {
                var line = Text(Lines.Catalog(Proc(), new Rows.CatalogRow("x", kind, "", "F.cs", 1)));
                Check.Yes(line.Contains("\"type\":\"" + KindInfo.WireName(kind) + "\",\"must_hit\":false", System.StringComparison.Ordinal), line);
            }
        });
        s.Add("lines: the tail of a long path is kept", () =>
        {
            var path = new string('d', 400) + "/Tail.cs";
            var line = Text(Lines.Catalog(Proc(), new Rows.CatalogRow("x", Kind.Always, "", path, 1)));
            Check.Yes(line.Contains("/Tail.cs\"", System.StringComparison.Ordinal), line);
            Check.No(line.Contains(new string('d', 300), System.StringComparison.Ordinal), "the head of the path");
        });
        s.Add("lines: an oversized text costs the text, not the line", () =>
        {
            var row = new Rows.CatalogRow(new string('i', 128), Kind.Always, new string('"', 512), new string('"', 256), 1);
            var line = Lines.Catalog(Proc(new string('t', 64), true, 7, 0), row, 400);
            Check.Yes(line.Length <= 400, "fits");
            Check.Yes(Text(line).Contains("\"id\":\"" + new string('i', 128), System.StringComparison.Ordinal), "the id survives");
            Check.No(Text(line).Contains("\"msg\"", System.StringComparison.Ordinal), "the message is dropped");
            Check.Raises<LineTooLongException>(() => Lines.Catalog(Proc(new string('t', 64), true, 7, 0), row, 100));
        });
        s.Add("lines: a condition pads both witnesses", () =>
        {
            var row = new Rows.AssertRow("basic.sometimes", Kind.Sometimes, 2, 2, Witness("t#1", 7, 0, 0, 0, 0), Witness("t#1", 8, 1, 0, 0, 0));
            var expected = "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"assert\",\"proc\":\"p1\",\"id\":\"basic.sometimes\"," +
                "\"hits\":" + W(4) + ",\"trues\":" + W(2) + ",\"falses\":" + W(2) + "," +
                "\"first_false\":" + Wit("{\"thread\":\"t#1\",\"ord\":" + W(8) + ",\"vals\":[1,0,0,0]}") + "," +
                "\"first_true\":" + Wit("{\"thread\":\"t#1\",\"ord\":" + W(7) + ",\"vals\":[0,0,0,0]}") + "}\n";
            Check.Equal(expected, Text(Lines.AssertLine(Proc(), row)));
        });
        s.Add("lines: a missing witness is null padded to the same width", () =>
        {
            var trueOnly = new Rows.AssertRow("a", Kind.Always, 5, 0, Witness("t#1", 1, 0, 2, 0, 0), null);
            var both = new Rows.AssertRow("a", Kind.Always, 5, 1, Witness("t#1", 1, 0, 2, 0, 0), Witness("t#1", 99, -1, -2, -3, -4));
            var a = Lines.AssertLine(Proc(), trueOnly);
            var b = Lines.AssertLine(Proc(), both);
            Check.Yes(Text(a).Contains("\"first_false\":" + Wit("null") + ",\"first_true\":", System.StringComparison.Ordinal), Text(a));
            Check.Equal(a.Length, b.Length);
        });
        s.Add("lines: a point kind sends only the true side", () =>
        {
            var row = new Rows.AssertRow("basic.reach.hit", Kind.Reachable, 1, 9, Witness("t#1", 11, 0, 0, 0, 0), null);
            var line = Text(Lines.AssertLine(Proc(), row));
            Check.No(line.Contains("first_false", System.StringComparison.Ordinal), "first_false");
            Check.Yes(line.Contains("\"hits\":" + W(1) + ",\"trues\":" + W(1) + ",\"falses\":" + W(0), System.StringComparison.Ordinal), line);
        });
        s.Add("lines: a witness is never sent behind a zero count", () =>
        {
            var row = new Rows.AssertRow("a", Kind.Always, 0, 0, Witness("t#1", 1, 0, 0, 0, 0), Witness("t#1", 2, 0, 0, 0, 0));
            var line = Text(Lines.AssertLine(Proc(), row));
            Check.Yes(line.Contains("\"first_false\":" + Wit("null") + ",\"first_true\":" + Wit("null") + "}", System.StringComparison.Ordinal), line);
        });
        s.Add("lines: work and end use fixed fields", () =>
        {
            Check.Equal(
                "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"work\",\"proc\":\"p1\",\"ops\":" + W(300) + "}\n",
                Text(Lines.Work(Proc(), 300)));
            Check.Equal(
                "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"end\",\"proc\":\"p1\",\"reason\":\"exit\"," +
                "\"status\":" + W(0) + ",\"sites\":7,\"evals\":" + W(11) + ",\"dropped\":" + W(0) + ",\"threads\":1}\n",
                Text(Lines.End(Proc(), EndReason.Exit, 0, 11, 1)));
        });
        s.Add("lines: end caps sites at what a parser accepts and names the reason", () =>
        {
            var big = Proc("p1", true, 9000, 0);
            var line = Text(Lines.End(big, EndReason.Signal, 11, 0, 0));
            Check.Yes(line.Contains("\"reason\":\"signal\"", System.StringComparison.Ordinal), line);
            Check.Yes(line.Contains("\"sites\":4096,", System.StringComparison.Ordinal), line);
            Check.Yes(Text(Lines.End(big, EndReason.Explicit, -3, 0, 0)).Contains("\"reason\":\"explicit\",\"status\":" + W(-3), System.StringComparison.Ordinal), "status");
        });
        s.Add("lines: end reports the drop count it is given", () =>
            Check.Yes(Text(Lines.End(Proc("p1", true, 7, 3), EndReason.Exit, 0, 0, 0)).Contains("\"dropped\":" + W(3), System.StringComparison.Ordinal), "dropped"));
        s.Add("lines: thread pads the key and the stability flag", () =>
        {
            var unstable = Text(Lines.Thread(Proc(), new Rows.ThreadRow("t#1", 11, 0, false)));
            Check.Equal(
                "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"thread\",\"proc\":\"p1\",\"thread\":\"t#1\"" +
                new string(' ', Fmt.ThreadKeyMax - 3) +
                ",\"evals\":" + W(11) + ",\"tape_consumed\":" + W(0) + ",\"key_stable\":false}\n",
                unstable);
            var stable = Text(Lines.Thread(Proc(), new Rows.ThreadRow("w-1", 1, 0, true)));
            Check.Yes(stable.EndsWith(",\"key_stable\":true }\n", System.StringComparison.Ordinal), stable);
        });
        s.Add("lines: a line that does not fit is refused, not truncated", () =>
            Check.Raises<LineTooLongException>(() => Lines.Hello(Proc(new string('t', 64), true, 7, 0), 60)));
        s.Add("lines: every line stays inside the cap at the protocol limits", () =>
        {
            long[] low = [long.MinValue, long.MinValue, long.MinValue, long.MinValue];
            long[] high = [long.MaxValue, long.MaxValue, long.MaxValue, long.MaxValue];
            var row = new Rows.AssertRow(
                new string('i', 128), Kind.Always, ulong.MaxValue, ulong.MaxValue,
                new Rows.Witness(new string('k', 64), ulong.MaxValue, low), new Rows.Witness(new string('k', 64), ulong.MaxValue, high));
            Check.Yes(Lines.AssertLine(Proc(new string('t', 64), true, 7, 0), row).Length <= 4096, "within 4096 bytes");
        });
    }
}
