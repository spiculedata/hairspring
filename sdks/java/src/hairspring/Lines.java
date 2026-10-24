package hairspring;

import java.nio.charset.StandardCharsets;

/**
 * One function per protocol line kind. Each returns the prefix, one JSON object and a newline as bytes; counters sit in
 * fixed-width fields so a line's length never depends on a counter's value. These are pure functions of immutable
 * values.
 */
final class Lines {
    static final String PREFIX = "HAIRSPRING_EVENT ";
    /** The protocol's per-process limit on ids; {@code end.sites} never claims more than a parser can accept. */
    static final int MAX_IDS = 4096;

    private static final String NULL_WITNESS = "null";

    private Lines() {}

    private static byte[] finish(StringBuilder body, int maxLen) {
        byte[] line = body.append("}\n").toString().getBytes(StandardCharsets.US_ASCII);
        if (line.length > maxLen) {
            throw new LineTooLongException("a " + line.length + " byte line passes the " + maxLen + " byte cap");
        }
        return line;
    }

    private static StringBuilder open(String kind) {
        return new StringBuilder(PREFIX).append("{\"v\":1,\"kind\":\"").append(kind).append('"');
    }

    private static StringBuilder proc(StringBuilder body, Rows.Proc p) {
        return body.append(",\"proc\":").append(Fmt.ident(p.tag(), Fmt.TAG_MAX));
    }

    private static String bool(boolean value) {
        return value ? "true" : "false";
    }

    static byte[] hello(Rows.Proc p) {
        return hello(p, Fmt.LINE_MAX);
    }

    static byte[] hello(Rows.Proc p, int maxLen) {
        StringBuilder body = open("hello")
                .append(",\"sdk\":\"").append(SdkInfo.NAME)
                .append("\",\"sdk_version\":\"").append(SdkInfo.VERSION).append('"');
        proc(body, p)
                .append(",\"catalog_hash\":\"").append(Fmt.hex64(p.catalogHash()))
                .append("\",\"signal_handlers\":").append(bool(p.signalHandlers()));
        if (!p.tagStable()) {
            body.append(",\"tag_stable\":false"); // additive: a tag derived from a pid
        }
        return finish(body, maxLen);
    }

    /** The source path may be longer than the protocol allows; keep its tail, which names the file. */
    private static byte[] fileTail(String path) {
        byte[] data = Fmt.utf8(path);
        int from = Math.max(0, data.length - Fmt.FILE_MAX);
        return java.util.Arrays.copyOfRange(data, from, data.length);
    }

    private static StringBuilder catalogBody(Rows.Proc p, Rows.CatalogRow row, boolean withText) {
        StringBuilder body = proc(open("catalog"), p)
                .append(",\"id\":").append(Fmt.ident(row.id(), Fmt.ID_MAX))
                .append(",\"type\":\"").append(row.kind().wireName())
                .append("\",\"must_hit\":").append(bool(row.kind().mustHit()));
        if (withText) {
            body.append(",\"msg\":").append(Fmt.jsonText(row.msg(), Fmt.MSG_MAX))
                    .append(",\"loc\":{\"file\":").append(Fmt.jsonText(fileTail(row.file()), Fmt.FILE_MAX))
                    .append(",\"line\":").append(row.line()).append('}');
        }
        return body;
    }

    static byte[] catalog(Rows.Proc p, Rows.CatalogRow row) {
        return catalog(p, row, Fmt.LINE_MAX);
    }

    static byte[] catalog(Rows.Proc p, Rows.CatalogRow row, int maxLen) {
        try {
            return finish(catalogBody(p, row, true), maxLen);
        } catch (LineTooLongException tooLong) {
            // An oversized message or path must not cost the whole line: retry with the text left out.
            return finish(catalogBody(p, row, false), maxLen);
        }
    }

    /**
     * A witness object padded to {@link Fmt#WITNESS_WIDTH}, or {@code null} padded the same, so presence does not change
     * length. A witness behind a zero count is never sent: the protocol rejects it.
     */
    private static String witness(Rows.Witness w, long count) {
        String text;
        if (w == null || count == 0) {
            text = NULL_WITNESS;
        } else {
            StringBuilder vals = new StringBuilder();
            for (int i = 0; i < w.vals().length; i++) {
                vals.append(i == 0 ? "" : ",").append(w.vals()[i]);
            }
            text = "{\"thread\":" + Fmt.threadKey(w.key(), Fmt.THREAD_KEY_MAX)
                    + ",\"ord\":" + Fmt.numW(w.ord()) + ",\"vals\":[" + vals + "]}";
        }
        return text.length() >= Fmt.WITNESS_WIDTH ? text : text + " ".repeat(Fmt.WITNESS_WIDTH - text.length());
    }

    static byte[] assertLine(Rows.Proc p, Rows.AssertRow row) {
        return assertLine(p, row, Fmt.LINE_MAX);
    }

    static byte[] assertLine(Rows.Proc p, Rows.AssertRow row, int maxLen) {
        long falses = row.kind().isPoint() ? 0 : row.falses();
        StringBuilder body = proc(open("assert"), p)
                .append(",\"id\":").append(Fmt.ident(row.id(), Fmt.ID_MAX))
                .append(",\"hits\":").append(Fmt.numW(row.trues() + falses))
                .append(",\"trues\":").append(Fmt.numW(row.trues()))
                .append(",\"falses\":").append(Fmt.numW(falses));
        if (!row.kind().isPoint()) {
            body.append(",\"first_false\":").append(witness(row.firstFalse(), falses));
        }
        body.append(",\"first_true\":").append(witness(row.firstTrue(), row.trues()));
        return finish(body, maxLen);
    }

    static byte[] work(Rows.Proc p, long ops) {
        return finish(proc(open("work"), p).append(",\"ops\":").append(Fmt.numW(ops)), Fmt.LINE_MAX);
    }

    static byte[] thread(Rows.Proc p, Rows.ThreadRow row) {
        StringBuilder body = proc(open("thread"), p)
                .append(",\"thread\":").append(Fmt.threadKeyPadded(row.key()))
                .append(",\"evals\":").append(Fmt.numW(row.evals()))
                .append(",\"tape_consumed\":").append(Fmt.numW(row.tapeConsumed()))
                .append(row.stable() ? ",\"key_stable\":true " : ",\"key_stable\":false");
        return finish(body, Fmt.LINE_MAX);
    }

    static byte[] end(Rows.Proc p, EndReason reason, long status, long evals, int threads) {
        StringBuilder body = proc(open("end"), p)
                .append(",\"reason\":\"").append(reason.wire())
                .append("\",\"status\":").append(Fmt.i64W(status))
                .append(",\"sites\":").append(Math.min(p.nIds(), MAX_IDS))
                .append(",\"evals\":").append(Fmt.numW(evals))
                .append(",\"dropped\":").append(Fmt.numW(p.dropped()))
                .append(",\"threads\":").append(threads);
        return finish(body, Fmt.LINE_MAX);
    }
}
