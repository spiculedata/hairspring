/* hs_internal.h -- state shared by the SDK's own files and its tests. Not part of the public API. */
#ifndef HS_INTERNAL_H
#define HS_INTERNAL_H

#include "../include/hairspring.h"
#include "hs_fmt.h"

#define HS_MAX_IDS 4096
#define HS_TAG_MAX 64
#define HS_OVERFLOW_SLOT (HS_MAX_THREADS - 1)
#define HS_WITNESS_WIDTH 197 /* widest witness object: thread 64 bytes, ord 20 digits, four 20-char values */

enum hs_end_reason { HS_END_EXIT = 0, HS_END_SIGNAL = 1, HS_END_EXPLICIT = 2 };

/* Consumer of finished lines (prefix, JSON and newline). Default: one write(2) to the events fd. */
typedef void (*hs_sink_fn)(const char *line, size_t len);

struct hs_proc {
  char tag[HS_TAG_MAX + 1];
  uint32_t forks;           /* forks so far from this process; names the next child */
  int fd;                   /* -1 when output is off */
  int signal_handlers;      /* 1 when HAIRSPRING_SIGNAL_REPORT=1 installed them */
  int status;               /* recorded by hs_exit, 0 otherwise */
  uint64_t dropped;         /* threads without a slot of their own, ids over the limit, failed or oversized lines */
  uint64_t catalog_hash;    /* order-independent hash of the distinct (id, kind) pairs */
  uint64_t n_ids;           /* distinct ids in the catalog */
  hs_sink_fn sink;          /* NULL: write to fd */
};

extern struct hs_proc hs_proc;
extern struct hs_slot hs_slots[HS_MAX_THREADS];
extern uint64_t hs_claims; /* slot claims so far; may exceed HS_MAX_THREADS */

/* Startup: environment, output fd, catalog index, handlers, hello and catalog lines. Idempotent. */
void hs_init(void);
/* Number of slots in use (claims, capped at the table size). */
size_t hs_slots_used(void);

/* Catalog: the records in the hs_catalog_v1 section, minus the sentinel. */
struct hs_site *hs_sites_begin(void);
struct hs_site *hs_sites_end(void);
static inline int hs_site_live(const struct hs_site *s) { return s->kind != HS_KIND_NONE; }
/* Index the records: set `canon` (first record per id), count distinct ids, hash. Safe to repeat.
** Returns how many distinct ids did not fit under HS_MAX_IDS (their records have a NULL canon). */
uint64_t hs_catalog_build(void);
/* The offline JSON twins: consecutive NUL-terminated strings in hs_catalog_str_v1. */
const char *hs_offline_begin(void);
const char *hs_offline_end(void);
/* Add every record's counts into its canonical record and choose witnesses. Report time only. */
void hs_catalog_sum(void);
/* Zero every record's counters (the fork child starts a fresh process). */
void hs_catalog_reset(void);

/* Lines: each fills `b` with prefix, JSON and newline; 0 on success, -1 if it did not fit. */
int hs_line_hello(struct hs_buf *b);
int hs_line_catalog(struct hs_buf *b, const struct hs_site *s);
int hs_line_assert(struct hs_buf *b, const struct hs_site *s);
int hs_line_work(struct hs_buf *b, uint64_t ops);
int hs_line_thread(struct hs_buf *b, const struct hs_slot *sl);
int hs_line_end(struct hs_buf *b, enum hs_end_reason reason, int64_t status, uint64_t evals);
int hs_line_eval(struct hs_buf *b, const char *id, const char *type, int ok);
const char *hs_kind_name(uint32_t kind);

/* Output and report. */
void hs_emit_line(const struct hs_buf *b);
void hs_emit_startup(void);
void hs_report_now(enum hs_end_reason reason, int64_t status);
void hs_report_rearm(void); /* the fork child may report again */
void hs_install_signal_handlers(void);

/* Test seam: route every line to `fn` instead of the fd. */
void hs_test_set_sink(hs_sink_fn fn);

#endif
