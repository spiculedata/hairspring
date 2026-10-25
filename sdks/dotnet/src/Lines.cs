using System;
using System.Text;

namespace Hairspring;

/// <summary>
/// One function per protocol line kind. Each returns the prefix, one JSON object and a newline as ASCII bytes; counters sit
/// in fixed-width fields so a line's length never depends on a counter's value. These are pure functions of immutable values.
/// </summary>
internal static class Lines
{
    public const string Prefix = "HAIRSPRING_EVENT ";

    /// <summary>The protocol's per-process limit on ids; <c>end.sites</c> never claims more than a parser can accept.</summary>
    public const int MaxIds = 4096;

    private const string NullWitness = "null";

    private static byte[] Finish(StringBuilder body, int maxLen)
    {
        var line = Encoding.ASCII.GetBytes(body.Append("}\n").ToString());
        if (line.Length > maxLen)
        {
            throw new LineTooLongException($"a {line.Length} byte line passes the {maxLen} byte cap");
        }
        return line;
    }

    private static StringBuilder Open(string kind) => new StringBuilder(Prefix).Append("{\"v\":1,\"kind\":\"").Append(kind).Append('"');

    private static StringBuilder ProcField(StringBuilder body, Rows.Proc p) =>
        body.Append(",\"proc\":").Append(Fmt.Ident(p.Tag, Fmt.TagMax));

    private static string Bool(bool value) => value ? "true" : "false";

    public static byte[] Hello(Rows.Proc p, int maxLen = Fmt.LineMax)
    {
        var body = Open("hello")
            .Append(",\"sdk\":\"").Append(SdkInfo.Name)
            .Append("\",\"sdk_version\":\"").Append(SdkInfo.Version).Append('"');
        ProcField(body, p)
            .Append(",\"catalog_hash\":\"").Append(Fmt.Hex64(p.CatalogHash))
            .Append("\",\"signal_handlers\":").Append(Bool(p.SignalHandlers));
        if (!p.TagStable)
        {
            body.Append(",\"tag_stable\":false"); // additive: a tag derived from a pid
        }
        return Finish(body, maxLen);
    }

    /// <summary>The source path may be longer than the protocol allows; keep its tail, which names the file.</summary>
    private static byte[] FileTail(string path)
    {
        var data = Fmt.Utf8(path);
        var from = Math.Max(0, data.Length - Fmt.FileMax);
        return data[from..];
    }

    private static StringBuilder CatalogBody(Rows.Proc p, Rows.CatalogRow row, bool withText)
    {
        var body = ProcField(Open("catalog"), p)
            .Append(",\"id\":").Append(Fmt.Ident(row.Id, Fmt.IdMax))
            .Append(",\"type\":\"").Append(KindInfo.WireName(row.Kind))
            .Append("\",\"must_hit\":").Append(Bool(KindInfo.MustHit(row.Kind)));
        if (withText)
        {
            body.Append(",\"msg\":").Append(Fmt.JsonText(row.Msg, Fmt.MsgMax))
                .Append(",\"loc\":{\"file\":").Append(Fmt.JsonText(FileTail(row.File), Fmt.FileMax))
                .Append(",\"line\":").Append(Fmt.Signed(Math.Max(0, row.Line))).Append('}');
        }
        return body;
    }

    public static byte[] Catalog(Rows.Proc p, Rows.CatalogRow row, int maxLen = Fmt.LineMax)
    {
        try
        {
            return Finish(CatalogBody(p, row, true), maxLen);
        }
        catch (LineTooLongException)
        {
            // An oversized message or path must not cost the whole line: retry with the text left out.
            return Finish(CatalogBody(p, row, false), maxLen);
        }
    }

    /// <summary>
    /// A witness object padded to <see cref="Fmt.WitnessWidth"/>, or <c>null</c> padded the same, so presence does not change
    /// length. A witness behind a zero count is never sent: the protocol rejects it.
    /// </summary>
    private static string WitnessText(Rows.Witness? w, ulong count)
    {
        string text;
        if (w is null || count == 0)
        {
            text = NullWitness;
        }
        else
        {
            var vals = new StringBuilder();
            for (var i = 0; i < w.Vals.Length; i++)
            {
                vals.Append(i == 0 ? "" : ",").Append(Fmt.Signed(w.Vals[i]));
            }
            text = "{\"thread\":" + Fmt.ThreadKey(w.Key, Fmt.ThreadKeyMax)
                + ",\"ord\":" + Fmt.NumW(w.Ord) + ",\"vals\":[" + vals + "]}";
        }
        return text.PadRight(Fmt.WitnessWidth);
    }

    public static byte[] AssertLine(Rows.Proc p, Rows.AssertRow row, int maxLen = Fmt.LineMax)
    {
        var point = KindInfo.IsPoint(row.Kind);
        var falses = point ? 0UL : row.Falses;
        var body = ProcField(Open("assert"), p)
            .Append(",\"id\":").Append(Fmt.Ident(row.Id, Fmt.IdMax))
            .Append(",\"hits\":").Append(Fmt.NumW(Saturating.Add(row.Trues, falses)))
            .Append(",\"trues\":").Append(Fmt.NumW(row.Trues))
            .Append(",\"falses\":").Append(Fmt.NumW(falses));
        if (!point)
        {
            body.Append(",\"first_false\":").Append(WitnessText(row.FirstFalse, falses));
        }
        body.Append(",\"first_true\":").Append(WitnessText(row.FirstTrue, row.Trues));
        return Finish(body, maxLen);
    }

    public static byte[] Work(Rows.Proc p, ulong ops) =>
        Finish(ProcField(Open("work"), p).Append(",\"ops\":").Append(Fmt.NumW(ops)), Fmt.LineMax);

    public static byte[] Thread(Rows.Proc p, Rows.ThreadRow row)
    {
        var body = ProcField(Open("thread"), p)
            .Append(",\"thread\":").Append(Fmt.ThreadKeyPadded(row.Key))
            .Append(",\"evals\":").Append(Fmt.NumW(row.Evals))
            .Append(",\"tape_consumed\":").Append(Fmt.NumW(row.TapeConsumed))
            .Append(row.Stable ? ",\"key_stable\":true " : ",\"key_stable\":false");
        return Finish(body, Fmt.LineMax);
    }

    public static byte[] End(Rows.Proc p, EndReason reason, long status, ulong evals, int threads)
    {
        var body = ProcField(Open("end"), p)
            .Append(",\"reason\":\"").Append(EndReasonInfo.Wire(reason))
            .Append("\",\"status\":").Append(Fmt.I64W(status))
            .Append(",\"sites\":").Append(Fmt.Signed(Math.Min(p.NIds, MaxIds)))
            .Append(",\"evals\":").Append(Fmt.NumW(evals))
            .Append(",\"dropped\":").Append(Fmt.NumW(p.Dropped))
            .Append(",\"threads\":").Append(Fmt.Signed(threads));
        return Finish(body, Fmt.LineMax);
    }
}
