/* hs_audit.c -- does the catalog account for every evaluation?
**
** Every evaluation made through a call-site macro adds one to the calling thread's `evals` and exactly one to the
** `trues` or `falses` of that site's record. If every record is in the catalog section, the sum of the records'
** counters therefore equals the sum of the threads' `evals`. A record the compiler placed elsewhere (some C++
** template call sites under g++, see docs/c-sdk.md) still counts evaluations in `evals` but is not found by the
** section walk, so the totals disagree and the shortfall says how many evaluations were never attributed.
**
** The audit runs only at report time, never at a call site. */
#include "hs_internal.h"

uint64_t hs_catalog_counted(void) {
  uint64_t total = 0;
  for (const struct hs_site *s = hs_sites_begin(); s < hs_sites_end(); s++) {
    if (!hs_site_live(s)) continue;
    total += __atomic_load_n(&s->trues, __ATOMIC_RELAXED) + __atomic_load_n(&s->falses, __ATOMIC_RELAXED);
  }
  return total;
}

/* A thread counts an evaluation before it updates the record, so a thread caught between the two makes `evals` run
** ahead by one. `tolerance` is the number of threads that can be in that state; the shortfall must exceed it. A
** catalog total above `evals` is not a loss: `evals` of a slot shared by many threads is a racy count. */
uint64_t hs_audit_missing(uint64_t evals, uint64_t counted, uint64_t tolerance) {
  if (evals <= counted) return 0;
  uint64_t deficit = evals - counted;
  return deficit > tolerance ? deficit : 0;
}

static uint64_t evals_now(void) {
  uint64_t total = 0;
  size_t used = hs_slots_used();
  for (size_t i = 0; i < used; i++) total += __atomic_load_n(&hs_slots[i].evals, __ATOMIC_RELAXED);
  return total;
}

/* How many threads can be inside a call site, between its two counter updates, while the audit reads. If no thread
** evaluated anything during the reads (`quiet`), only the thread a signal interrupted can be. Otherwise any thread
** but the reporting one can be, and the threads that have already finished are counted too: that makes a small loss
** invisible in a busy multi-threaded exit, and it never makes a complete catalog look incomplete. */
static uint64_t in_flight_bound(enum hs_end_reason reason, uint64_t threads, int quiet) {
  if (quiet) return reason == HS_END_SIGNAL ? 1u : 0u;
  if (reason == HS_END_SIGNAL || threads == 0) return threads;
  return threads - 1u;
}

static void announce(uint64_t missing) {
  char store[192];
  struct hs_buf b;
  hs_buf_init(&b, store, sizeof store);
  hs_put_str(&b, "hairspring: incomplete catalog: ");
  hs_put_u64(&b, missing);
  hs_put_str(&b, " evaluation(s) ran at call sites that are missing from the catalog section\n");
  if (b.over) {
    __atomic_fetch_add(&hs_proc.dropped, 1u, __ATOMIC_RELAXED);
    return;
  }
  hs_emit_line(&b);
}

uint64_t hs_audit_run(enum hs_end_reason reason, uint64_t threads) {
  /* With more threads than slots the last slot is shared and its count is racy, so the comparison means nothing. */
  if (hs_slots_used() >= HS_MAX_THREADS) return 0;
  uint64_t before = evals_now();
  uint64_t counted = hs_catalog_counted(); /* read after `before`: a surplus here can only hide a loss, never invent one */
  int quiet = evals_now() == before;
  uint64_t missing = hs_audit_missing(before, counted, in_flight_bound(reason, threads, quiet));
  if (missing == 0) return 0;
  __atomic_fetch_add(&hs_proc.dropped, 1u, __ATOMIC_RELAXED);
  announce(missing);
  return missing;
}
