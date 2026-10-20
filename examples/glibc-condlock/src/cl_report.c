#include "cl_report.h"

#include <stdio.h>

#define LL(x) ((unsigned long long)(x))

static int finish(int written, size_t len) { return (written < 0 || (size_t)written >= len) ? -1 : written; }

int cl_report_format(char *buf, size_t len, const struct cl_report *r) {
  int n = snprintf(buf, len,
                   "condlock libc=%s hdr=%s rounds=%8llu signals=%8llu blocked1=%8llu maxb=%8llu maxstag=%8llu "
                   "ticks=%8llu spanned=%8llu",
                   r->libc, r->hdr, LL(r->rounds_done), LL(r->signals), LL(r->signals_blocked), LL(r->max_blocked),
                   LL(r->max_stagnant), LL(r->ticks), LL(r->spanned));
  n = finish(n, len);
  if (n < 0 || !r->has_applied) return n;
  int m = snprintf(buf + n, len - (size_t)n, " applied=%d", r->applied ? 1 : 0);
  return finish(m, len - (size_t)n) < 0 ? -1 : n + m;
}

static const char *subtype_name(unsigned base) {
  switch (base) {
    case CL_SUB_Q: return "Q";
    case CL_SUB_L: return "L";
    case CL_SUB_B: return "B";
    case CL_SUB_M: return "M";
    default: return "NONE";
  }
}

/* Raw condvar fields, appended after the counters. Arm NONE has none. */
static int format_raw(char *buf, size_t len, const struct cl_condraw *c, uint32_t waiting_n) {
  if (c->arm == CL_ARM_NONE) return snprintf(buf, len, " cond=none");
  int g1 = (int)((c->wseq & 1u) ^ 1u);
  int n = snprintf(buf, len, " cond=%s wseq=%llu g1_start=%llu g1=%d", c->arm == CL_ARM_U ? "U" : "C", LL(c->wseq),
                   LL(c->g1_start), g1);
  if (n < 0 || (size_t)n >= len) return -1;
  if (c->arm == CL_ARM_U) {
    int m = snprintf(buf + n, len - (size_t)n, " g_refs=%u,%u", (unsigned)c->g_refs[0], (unsigned)c->g_refs[1]);
    if (m < 0 || (size_t)m >= len - (size_t)n) return -1;
    n += m;
  }
  int m = snprintf(buf + n, len - (size_t)n, " g_size=%u,%u g1_orig_size=%u wrefs>>3=%u wrefs_match=%d g_signals=%u,%u",
                   (unsigned)c->g_size[0], (unsigned)c->g_size[1], (unsigned)c->g1_orig_size, (unsigned)(c->wrefs >> 3),
                   (c->wrefs >> 3) == waiting_n ? 1 : 0, (unsigned)c->g_signals[0], (unsigned)c->g_signals[1]);
  if (m < 0 || (size_t)m >= len - (size_t)n) return -1;
  return n + m;
}

int cl_dump_format(char *buf, size_t len, const struct cl_dump *d) {
  unsigned base = (unsigned)d->subtype & ~(unsigned)CL_SUB_ATTRIBUTED;
  int attributed = ((unsigned)d->subtype & CL_SUB_ATTRIBUTED) != 0;
  int n = snprintf(buf, len,
                   "condlock: %s subtype=%s%s s_seq=%llu blocked=%u waiting=%u locked=%d progress=%llu stagnant=%u "
                   "ticks=%llu",
                   d->what, subtype_name(base), attributed ? "+attributed" : "", LL(d->s_seq), (unsigned)d->blocked_n,
                   (unsigned)d->waiting_n, d->is_locked ? 1 : 0, LL(d->progress), (unsigned)d->stagnant, LL(d->ticks));
  if (n < 0 || (size_t)n >= len) return -1;
  int m = format_raw(buf + n, len - (size_t)n, &d->raw, d->waiting_n);
  if (m < 0 || (size_t)m >= len - (size_t)n) return -1;
  return n + m;
}
