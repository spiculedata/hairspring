package hairspring;

import java.nio.charset.StandardCharsets;

/** One function per protocol line kind: exact text, fixed widths, no witness behind a zero count. */
final class LinesTests {
    private LinesTests() {}

    private static Rows.Proc proc() {
        return proc("p1", true, 7, 0);
    }

    private static Rows.Proc proc(String tag, boolean stable, int ids, long dropped) {
        return new Rows.Proc(tag, stable, false, 0xFB6CF21A75AAD6EFL, ids, dropped);
    }

    private static String w(long n) {
        return String.format("%20d", n);
    }

    private static String wit(String text) {
        return String.format("%-" + Fmt.WITNESS_WIDTH + "s", text);
    }

    private static String text(byte[] line) {
        return new String(line, StandardCharsets.US_ASCII);
    }

    private static Rows.Witness witness(String key, long ord, long... vals) {
        return new Rows.Witness(key, ord, vals);
    }

    static void register(Suite s) {
        s.add("lines: hello names the sdk and the catalog hash", () -> {
            Check.equal(
                    "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"hello\",\"sdk\":\"java\",\"sdk_version\":\""
                            + SdkInfo.VERSION
                            + "\",\"proc\":\"p1\",\"catalog_hash\":\"fb6cf21a75aad6ef\",\"signal_handlers\":false}\n",
                    text(Lines.hello(proc())));
        });
        s.add("lines: hello flags an unstable tag", () -> {
            String line = text(Lines.hello(proc("p1", false, 7, 0)));
            Check.yes(line.endsWith("\"signal_handlers\":false,\"tag_stable\":false}\n"), line);
        });
        s.add("lines: catalog carries text and location", () -> {
            Rows.CatalogRow row = new Rows.CatalogRow("basic.always.ok", Kind.ALWAYS, "always holds", "Sample.java", 31);
            Check.equal(
                    "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"catalog\",\"proc\":\"p1\",\"id\":\"basic.always.ok\","
                            + "\"type\":\"always\",\"must_hit\":true,\"msg\":\"always holds\","
                            + "\"loc\":{\"file\":\"Sample.java\",\"line\":31}}\n",
                    text(Lines.catalog(proc(), row)));
        });
        s.add("lines: must_hit is false for unreachable and always_or_unreachable", () -> {
            for (Kind kind : new Kind[] {Kind.UNREACHABLE, Kind.ALWAYS_OR_UNREACHABLE}) {
                String line = text(Lines.catalog(proc(), new Rows.CatalogRow("x", kind, "", "F.java", 1)));
                Check.yes(line.contains("\"type\":\"" + kind.wireName() + "\",\"must_hit\":false"), line);
            }
        });
        s.add("lines: the tail of a long path is kept", () -> {
            String path = "d".repeat(400) + "/Tail.java";
            String line = text(Lines.catalog(proc(), new Rows.CatalogRow("x", Kind.ALWAYS, "", path, 1)));
            Check.yes(line.contains("/Tail.java\""), line);
            Check.no(line.contains("d".repeat(300)), "the head of the path");
        });
        s.add("lines: an oversized text costs the text, not the line", () -> {
            Rows.CatalogRow row = new Rows.CatalogRow("i".repeat(128), Kind.ALWAYS, "\"".repeat(512), "\"".repeat(256), 1);
            byte[] line = Lines.catalog(proc("t".repeat(64), true, 7, 0), row, 400);
            Check.yes(line.length <= 400, "fits");
            Check.yes(text(line).contains("\"id\":\"" + "i".repeat(128)), "the id survives");
            Check.no(text(line).contains("\"msg\""), "the message is dropped");
            Check.raises(LineTooLongException.class, () -> Lines.catalog(proc("t".repeat(64), true, 7, 0), row, 100));
        });
        s.add("lines: a condition pads both witnesses", () -> {
            Rows.AssertRow row = new Rows.AssertRow(
                    "basic.sometimes", Kind.SOMETIMES, 2, 2, witness("t#1", 7, 0, 0, 0, 0), witness("t#1", 8, 1, 0, 0, 0));
            String expected = "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"assert\",\"proc\":\"p1\",\"id\":\"basic.sometimes\","
                    + "\"hits\":" + w(4) + ",\"trues\":" + w(2) + ",\"falses\":" + w(2) + ","
                    + "\"first_false\":" + wit("{\"thread\":\"t#1\",\"ord\":" + w(8) + ",\"vals\":[1,0,0,0]}") + ","
                    + "\"first_true\":" + wit("{\"thread\":\"t#1\",\"ord\":" + w(7) + ",\"vals\":[0,0,0,0]}") + "}\n";
            Check.equal(expected, text(Lines.assertLine(proc(), row)));
        });
        s.add("lines: a missing witness is null padded to the same width", () -> {
            Rows.AssertRow trueOnly = new Rows.AssertRow("a", Kind.ALWAYS, 5, 0, witness("t#1", 1, 0, 2, 0, 0), null);
            Rows.AssertRow both = new Rows.AssertRow(
                    "a", Kind.ALWAYS, 5, 1, witness("t#1", 1, 0, 2, 0, 0), witness("t#1", 99, -1, -2, -3, -4));
            byte[] a = Lines.assertLine(proc(), trueOnly);
            byte[] b = Lines.assertLine(proc(), both);
            Check.yes(text(a).contains("\"first_false\":" + wit("null") + ",\"first_true\":"), text(a));
            Check.equal(a.length, b.length);
        });
        s.add("lines: a point kind sends only the true side", () -> {
            Rows.AssertRow row = new Rows.AssertRow(
                    "basic.reach.hit", Kind.REACHABLE, 1, 9, witness("t#1", 11, 0, 0, 0, 0), null);
            String line = text(Lines.assertLine(proc(), row));
            Check.no(line.contains("first_false"), "first_false");
            Check.yes(line.contains("\"hits\":" + w(1) + ",\"trues\":" + w(1) + ",\"falses\":" + w(0)), line);
        });
        s.add("lines: a witness is never sent behind a zero count", () -> {
            Rows.AssertRow row = new Rows.AssertRow(
                    "a", Kind.ALWAYS, 0, 0, witness("t#1", 1, 0, 0, 0, 0), witness("t#1", 2, 0, 0, 0, 0));
            String line = text(Lines.assertLine(proc(), row));
            Check.yes(line.contains("\"first_false\":" + wit("null") + ",\"first_true\":" + wit("null") + "}"), line);
        });
        s.add("lines: work and end use fixed fields", () -> {
            Check.equal(
                    "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"work\",\"proc\":\"p1\",\"ops\":" + w(300) + "}\n",
                    text(Lines.work(proc(), 300)));
            Check.equal(
                    "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"end\",\"proc\":\"p1\",\"reason\":\"exit\","
                            + "\"status\":" + w(0) + ",\"sites\":7,\"evals\":" + w(11) + ",\"dropped\":" + w(0) + ",\"threads\":1}\n",
                    text(Lines.end(proc(), EndReason.EXIT, 0, 11, 1)));
        });
        s.add("lines: end caps sites at what a parser accepts and names the reason", () -> {
            Rows.Proc big = proc("p1", true, 9000, 0);
            String line = text(Lines.end(big, EndReason.SIGNAL, 11, 0, 0));
            Check.yes(line.contains("\"reason\":\"signal\""), line);
            Check.yes(line.contains("\"sites\":4096,"), line);
            Check.yes(text(Lines.end(big, EndReason.EXPLICIT, -3, 0, 0)).contains("\"reason\":\"explicit\",\"status\":" + w(-3)), "status");
        });
        s.add("lines: end reports the drop count it is given", () -> {
            Check.yes(text(Lines.end(proc("p1", true, 7, 3), EndReason.EXIT, 0, 0, 0)).contains("\"dropped\":" + w(3)), "dropped");
        });
        s.add("lines: thread pads the key and the stability flag", () -> {
            String unstable = text(Lines.thread(proc(), new Rows.ThreadRow("t#1", 11, 0, false)));
            Check.equal(
                    "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"thread\",\"proc\":\"p1\",\"thread\":\"t#1\""
                            + " ".repeat(Fmt.THREAD_KEY_MAX - 3)
                            + ",\"evals\":" + w(11) + ",\"tape_consumed\":" + w(0) + ",\"key_stable\":false}\n",
                    unstable);
            String stable = text(Lines.thread(proc(), new Rows.ThreadRow("w-1", 1, 0, true)));
            Check.yes(stable.endsWith(",\"key_stable\":true }\n"), stable);
        });
        s.add("lines: a line that does not fit is refused, not truncated", () -> {
            Check.raises(LineTooLongException.class, () -> Lines.hello(proc("t".repeat(64), true, 7, 0), 60));
        });
        s.add("lines: every line stays inside the cap at the protocol limits", () -> {
            long[] low = {Long.MIN_VALUE, Long.MIN_VALUE, Long.MIN_VALUE, Long.MIN_VALUE};
            long[] high = {Long.MAX_VALUE, Long.MAX_VALUE, Long.MAX_VALUE, Long.MAX_VALUE};
            Rows.AssertRow row = new Rows.AssertRow(
                    "i".repeat(128), Kind.ALWAYS, -1L, -1L,
                    new Rows.Witness("k".repeat(64), -1L, low), new Rows.Witness("k".repeat(64), -1L, high));
            Check.yes(Lines.assertLine(proc("t".repeat(64), true, 7, 0), row).length <= 4096, "within 4096 bytes");
        });
    }
}
