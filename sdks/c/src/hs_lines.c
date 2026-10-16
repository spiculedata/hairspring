/* hs_lines.c -- one function per protocol line kind. Each fills a buffer with the prefix, one JSON object and a
** newline; counters are right-aligned in fixed fields so a line's length never depends on counter values. */
#include <string.h>

#include "hs_internal.h"

#define PREFIX "HAIRSPRING_EVENT "

static int finish(struct hs_buf *b) {
  hs_put_str(b, "}\n");
  return b->over ? -1 : 0;
}

static void open_line(struct hs_buf *b, const char *kind) {
  hs_put_str(b, PREFIX "{\"v\":1,\"kind\":\"");
  hs_put_str(b, kind);
  hs_put_str(b, "\"");
}

static void put_proc(struct hs_buf *b) {
  hs_put_str(b, ",\"proc\":");
  hs_put_ident(b, hs_proc.tag, HS_TAG_MAX);
}

const char *hs_kind_name(uint32_t kind) {
  switch (kind) {
    case HS_KIND_ALWAYS: return "always";
    case HS_KIND_ALWAYS_OR_UNREACHABLE: return "always_or_unreachable";
    case HS_KIND_SOMETIMES: return "sometimes";
    case HS_KIND_REACHABLE: return "reachable";
    case HS_KIND_UNREACHABLE: return "unreachable";
    default: return "unknown";
  }
}

static int kind_must_hit(uint32_t kind) {
  return kind == HS_KIND_ALWAYS || kind == HS_KIND_SOMETIMES || kind == HS_KIND_REACHABLE;
}

static int is_point_kind(uint32_t kind) { return kind == HS_KIND_REACHABLE || kind == HS_KIND_UNREACHABLE; }

int hs_line_hello(struct hs_buf *b) {
  open_line(b, "hello");
  hs_put_str(b, ",\"sdk\":\"c\",\"sdk_version\":\"" HS_SDK_VERSION "\"");
  put_proc(b);
  hs_put_str(b, ",\"catalog_hash\":\"");
  hs_put_hex64(b, hs_proc.catalog_hash);
  hs_put_str(b, hs_proc.signal_handlers ? "\",\"signal_handlers\":true" : "\",\"signal_handlers\":false");
  return finish(b);
}

/* The source path may be longer than the protocol allows; keep its tail, which names the file. */
static const char *file_tail(const char *file) {
  size_t n = strlen(file);
  return n > HS_FILE_MAX ? file + (n - HS_FILE_MAX) : file;
}

static void catalog_body(struct hs_buf *b, const struct hs_site *s, int with_text) {
  open_line(b, "catalog");
  put_proc(b);
  hs_put_str(b, ",\"id\":");
  hs_put_ident(b, s->id, HS_ID_MAX);
  hs_put_str(b, ",\"type\":\"");
  hs_put_str(b, hs_kind_name(s->kind));
  hs_put_str(b, kind_must_hit(s->kind) ? "\",\"must_hit\":true" : "\",\"must_hit\":false");
  if (!with_text) return;
  hs_put_str(b, ",\"msg\":");
  hs_put_json_text(b, s->msg, HS_MSG_MAX);
  hs_put_str(b, ",\"loc\":{\"file\":");
  hs_put_json_text(b, file_tail(s->file), HS_FILE_MAX);
  hs_put_str(b, ",\"line\":");
  hs_put_u64(b, s->line);
  hs_put_str(b, "}");
}

int hs_line_catalog(struct hs_buf *b, const struct hs_site *s) {
  catalog_body(b, s, 1);
  if (!b->over) return finish(b);
  /* An oversized message or path must not cost the whole line: retry with the text left out. */
  b->len = 0;
  b->over = 0;
  catalog_body(b, s, 0);
  return finish(b);
}

/* A witness object padded to HS_WITNESS_WIDTH, or `null` padded the same, so presence does not change length. */
static void put_witness(struct hs_buf *b, const struct hs_witness *w) {
  size_t start = b->len;
  if (w && w->slot < HS_MAX_THREADS) {
    hs_put_str(b, "{\"thread\":");
    hs_put_thread_key(b, hs_slots[w->slot].key, HS_THREAD_KEY_MAX);
    hs_put_str(b, ",\"ord\":");
    hs_put_u64_w(b, w->ord);
    hs_put_str(b, ",\"vals\":[");
    for (int i = 0; i < 4; i++) {
      if (i) hs_put_str(b, ",");
      hs_put_i64(b, w->vals[i]);
    }
    hs_put_str(b, "]}");
  } else {
    hs_put_str(b, "null");
  }
  size_t used = b->len - start;
  hs_put_pad(b, used < HS_WITNESS_WIDTH ? HS_WITNESS_WIDTH - used : 0);
}

int hs_line_assert(struct hs_buf *b, const struct hs_site *s) {
  open_line(b, "assert");
  put_proc(b);
  hs_put_str(b, ",\"id\":");
  hs_put_ident(b, s->id, HS_ID_MAX);
  uint64_t falses = is_point_kind(s->kind) ? 0 : s->sum_falses;
  hs_put_str(b, ",\"hits\":");
  hs_put_u64_w(b, s->sum_trues + falses);
  hs_put_str(b, ",\"trues\":");
  hs_put_u64_w(b, s->sum_trues);
  hs_put_str(b, ",\"falses\":");
  hs_put_u64_w(b, falses);
  if (!is_point_kind(s->kind)) {
    hs_put_str(b, ",\"first_false\":");
    put_witness(b, falses ? s->pick_false : NULL);
  }
  hs_put_str(b, ",\"first_true\":");
  put_witness(b, s->sum_trues ? s->pick_true : NULL);
  return finish(b);
}

int hs_line_work(struct hs_buf *b, uint64_t ops) {
  open_line(b, "work");
  put_proc(b);
  hs_put_str(b, ",\"ops\":");
  hs_put_u64_w(b, ops);
  return finish(b);
}

int hs_line_thread(struct hs_buf *b, const struct hs_thread_totals *t) {
  open_line(b, "thread");
  put_proc(b);
  hs_put_str(b, ",\"thread\":");
  hs_put_thread_key_padded(b, t->slot->key);
  hs_put_str(b, ",\"evals\":");
  hs_put_u64_w(b, t->evals);
  hs_put_str(b, ",\"tape_consumed\":");
  hs_put_u64_w(b, t->tape_consumed);
  hs_put_str(b, t->is_stable ? ",\"key_stable\":true " : ",\"key_stable\":false");
  return finish(b);
}

int hs_line_end(struct hs_buf *b, enum hs_end_reason reason, int64_t status, uint64_t evals, uint64_t threads) {
  static const char *const names[] = {"exit", "signal", "explicit"};
  open_line(b, "end");
  put_proc(b);
  hs_put_str(b, ",\"reason\":\"");
  hs_put_str(b, names[reason]);
  hs_put_str(b, "\",\"status\":");
  hs_put_i64_w(b, status);
  hs_put_str(b, ",\"sites\":");
  hs_put_u64(b, hs_proc.n_ids < HS_MAX_IDS ? hs_proc.n_ids : HS_MAX_IDS); /* what a parser can accept */
  hs_put_str(b, ",\"evals\":");
  hs_put_u64_w(b, evals);
  hs_put_str(b, ",\"dropped\":");
  hs_put_u64_w(b, hs_proc.dropped);
  hs_put_str(b, ",\"threads\":");
  hs_put_u64(b, threads);
  return finish(b);
}

int hs_line_eval(struct hs_buf *b, const char *id, const char *type, int ok) {
  open_line(b, "eval");
  hs_put_str(b, ",\"id\":");
  hs_put_ident(b, id, HS_ID_MAX);
  hs_put_str(b, ",\"type\":\"");
  hs_put_str(b, type);
  hs_put_str(b, ok ? "\",\"ok\":true" : "\",\"ok\":false");
  return finish(b);
}
