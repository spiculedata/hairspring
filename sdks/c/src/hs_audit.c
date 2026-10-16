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

/* How many evaluations can be counted in a slot but not yet in a record: at most one per thread that is inside a call
** site between its two updates. The reporting thread is not (at exit or in hs_report), so it is excluded; after a
** fatal signal it may be (the signal can land between the two updates), so it is not. Finished threads are counted
** too, because the SDK cannot tell them from running ones. The guarantee is therefore one-sided: a complete catalog
** is not reported incomplete as long as only these evaluations are in flight (not covered: exit() called from a
** signal handler that interrupted a call site), and in return a small loss in a run with several threads can go
** unnoticed. With one thread the bound is 0 (1 after a signal) and every missing evaluation is reported. */
static uint64_t in_flight_bound(enum hs_end_reason reason, uint64_t threads) {
  if (reason == HS_END_SIGNAL || threads == 0) return threads;
  return threads - 1u;
}

static void announce(uint64_t missing) {
  char store[192]; /* the text is about 110 bytes with a 20-digit number: it always fits */
  struct hs_buf b;
  hs_buf_init(&b, store, sizeof store);
  hs_put_str(&b, "hairspring: incomplete catalog: ");
  hs_put_u64(&b, missing);
  hs_put_str(&b, " evaluation(s) ran at call sites that are missing from the catalog section\n");
  hs_emit_line(&b);
}

uint64_t hs_audit_run(enum hs_end_reason reason, uint64_t threads) {
  /* With more threads than slots the last slot is shared and its count is racy, so the comparison means nothing. */
  if (hs_slots_used() >= HS_MAX_THREADS) return 0;
  uint64_t evals = evals_now();
  uint64_t counted = hs_catalog_counted(); /* read after `evals`: a surplus here can only hide a loss, never invent one */
  uint64_t missing = hs_audit_missing(evals, counted, in_flight_bound(reason, threads));
  if (missing == 0) return 0;
  __atomic_fetch_add(&hs_proc.dropped, 1u, __ATOMIC_RELAXED);
  announce(missing);
  return missing;
}
