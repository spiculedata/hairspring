#include "cl_worker.h"

#include <stdio.h>

#include "cl_lock.h"
#include "cl_report.h"
#include "cl_sdk.h"

#define RELAXED memory_order_relaxed

void *cl_worker_main(void *arg) {
  struct cl_worker_arg *w = (struct cl_worker_arg *)arg;
  struct cl_ctx *c = w->ctx;
  hs_thread("w", w->id);
  for (unsigned r = 0; r < c->cfg.rounds; r++) {
    cl_acquire(c, w->id);
    HS_WORK(1);
    for (unsigned i = 0; i < CL_ROUND_INCREMENTS; i++) c->crit_counter = c->crit_counter + 1;
    cl_release(c, w->id);
    cl_barrier(c, w->id);
    atomic_fetch_add_explicit(&c->rounds_done, 1, RELAXED);
  }
  atomic_fetch_add_explicit(&c->finished, 1, RELAXED);
  return NULL;
}

#if defined(__GLIBC__)
#include <gnu/libc-version.h>
#define CL_STR2(x) #x
#define CL_STR(x) CL_STR2(x)
static const char *libc_name(void) { return gnu_get_libc_version(); }
static const char *hdr_name(void) { return CL_STR(__GLIBC__) "." CL_STR(__GLIBC_MINOR__); }
#else
static const char *libc_name(void) { return "none"; }
static const char *hdr_name(void) { return "none"; }
#endif

void cl_print_report(const struct cl_ctx *c) {
  struct cl_report r = {0};
  char line[CL_LINE_MAX];
  r.libc = libc_name();
  r.hdr = hdr_name();
  r.rounds_done = atomic_load_explicit(&c->rounds_done, RELAXED);
  r.signals = atomic_load_explicit(&c->st_signals, RELAXED);
  r.signals_blocked = atomic_load_explicit(&c->st_blocked1, RELAXED);
  r.max_blocked = atomic_load_explicit(&c->st_max_blocked, RELAXED);
  r.ticks = atomic_load_explicit(&c->st_ticks, RELAXED);
  r.spanned = atomic_load_explicit(&c->st_spanned, RELAXED);
  r.max_stagnant = (uint32_t)atomic_load_explicit(&c->st_max_stagnant, RELAXED);
  r.has_applied = CL_PLANTED;
  r.applied = (int)atomic_load_explicit(&c->plant_applied, RELAXED);
  if (cl_report_format(line, sizeof line, &r) < 0) return;
  fprintf(stdout, "%s\n", line);
  fflush(stdout);
}
