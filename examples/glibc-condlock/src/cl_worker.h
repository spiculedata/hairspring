/* cl_worker.h -- worker threads, the watchdog thread, and the stdout report. */
#ifndef CL_WORKER_H
#define CL_WORKER_H

#include "cl_ctx.h"

#define CL_ROUND_INCREMENTS 1000u /* the public reproducers' per-round critical-section work */

struct cl_worker_arg {
  struct cl_ctx *ctx;
  unsigned id;
};

/* Worker thread: R rounds of acquire, work, release, barrier. */
void *cl_worker_main(void *arg);

/* Watchdog thread (arg = struct cl_ctx *): ticks until ctx->stop; ends the process itself on a hit or stall. */
void *cl_watchdog_main(void *arg);

/* Prints the one-line report on stdout (and flushes). */
void cl_print_report(const struct cl_ctx *c);

#endif
