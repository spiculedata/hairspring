/* hs_internal.h -- state shared by the SDK's own files and its tests. Not part of the public API. */
#ifndef HS_INTERNAL_H
#define HS_INTERNAL_H

#include "../include/hairspring.h"
#include "hs_fmt.h"

#define HS_MAX_IDS 4096          /* the protocol's per-process limit; ids beyond it are still sent, so the parser degrades */
#define HS_MAX_DISTINCT 16384    /* what the catalog index can hold; beyond this ids are dropped and counted */
#define HS_TAG_MAX 64
#define HS_OVERFLOW_SLOT (HS_MAX_THREADS - 1)
#define HS_WITNESS_WIDTH 197 /* widest witness object: thread 64 bytes, ord 20 digits, four 20-char values */

enum hs_end_reason { HS_END_EXIT = 0, HS_END_SIGNAL = 1, HS_END_EXPLICIT = 2 };

/* Consumer of finished lines (prefix, JSON and newline). Default: write(2) to the events fd. */
typedef void (*hs_sink_fn)(const char *line, size_t len);

struct hs_proc {
  char tag[HS_TAG_MAX + 1];
  uint32_t forks;           /* forks so far from this process; names the next child */
  int fd;                   /* private descriptor for events; -1 when output is off or unavailable */
  int signal_handlers;      /* 1 when HAIRSPRING_SIGNAL_REPORT=1 installed every handler */
  int status;               /* recorded by hs_exit or on_exit, 0 otherwise */
  int tag_stable;           /* 0 for a tag derived from a pid (spawned child) and everything below it */
  int startup_pending;      /* a fork child sends hello and catalog when it reports, not before */
  uint64_t dropped;         /* threads without a slot of their own, ids lost, refused settings, failed lines */
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

/* Process identity (hs_identity.c). */
void hs_identity_init(void);
void hs_identity_child(uint32_t n);
void hs_identity_announced(void); /* hello has gone out: an exec without fork must take a new tag */

/* Catalog: the records in the hs_catalog_v1 section, plus one sentinel with kind HS_KIND_NONE. */
struct hs_site *hs_sites_begin(void);
struct hs_site *hs_sites_end(void);
static inline int hs_site_live(const struct hs_site *s) { return s->kind != HS_KIND_NONE; }
/* Index the records: set `canon` (first record per id), flag the records that need a catalog line, count distinct
** ids, hash. Safe to repeat. Returns how many records had no room in the index (their canon is NULL). */
uint64_t hs_catalog_build(void);
/* The offline JSON twins: consecutive NUL-terminated strings in hs_catalog_str_v1. */
const char *hs_offline_begin(void);
const char *hs_offline_end(void);
/* Add every record's counts into its canonical record and choose witnesses. Report time only. */
void hs_catalog_sum(void);
/* Zero every record's counters (the fork child starts a fresh process). */
void hs_catalog_reset(void);

/* One thread's totals, read once per report so the thread lines and the end line agree. */
struct hs_thread_totals {
  const struct hs_slot *slot;
  uint64_t evals;
  uint64_t work;
  uint64_t tape_consumed;
  uint64_t is_stable;
};

/* Lines: each fills `b` with prefix, JSON and newline; 0 on success, -1 if it did not fit. */
int hs_line_hello(struct hs_buf *b);
int hs_line_catalog(struct hs_buf *b, const struct hs_site *s);
int hs_line_assert(struct hs_buf *b, const struct hs_site *s);
int hs_line_work(struct hs_buf *b, uint64_t ops);
int hs_line_thread(struct hs_buf *b, const struct hs_thread_totals *t);
int hs_line_end(struct hs_buf *b, enum hs_end_reason reason, int64_t status, uint64_t evals, uint64_t threads);
int hs_line_eval(struct hs_buf *b, const char *id, const char *type, int ok);
const char *hs_kind_name(uint32_t kind);

/* Output and report. */
void hs_emit_line(const struct hs_buf *b);
void hs_emit_startup(void);
void hs_report_now(enum hs_end_reason reason, int64_t status);
int hs_events_fd_remember(int fd); /* note which file `fd` refers to, before it is published; -1 if unknown */
void hs_report_rearm(void); /* the fork child may report again */
/* Returns how many handlers could not be installed (0 = all installed). */
int hs_install_signal_handlers(void);

/* Test seam: route every line to `fn` instead of the fd. */
void hs_test_set_sink(hs_sink_fn fn);

#endif
