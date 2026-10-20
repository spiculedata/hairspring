/* condlock: N workers contend for a lock built from a mutex and a condvar (untimed pthread_cond_wait, an
 * unconditional pthread_cond_signal on release), meet at a barrier each round, and a watchdog decides whether a signal
 * was lost. Exit status: 0 completed, 3 property fired (HIT), 4 STALL, 5 harness invariant, 64 usage. Never 1. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cl_check.h"
#include "cl_config.h"
#include "cl_sdk.h"
#include "cl_worker.h"

#define RELAXED memory_order_relaxed

static void init_ctx(struct cl_ctx *c, const struct cl_config *cfg) {
  memset(c, 0, sizeof *c);
  c->cfg = *cfg;
  cl_ok(pthread_mutex_init(&c->g, NULL), "init G");
  cl_ok(pthread_cond_init(&c->l, NULL), "init L");
  cl_ok(pthread_mutex_init(&c->b, NULL), "init B");
  cl_ok(pthread_cond_init(&c->k, NULL), "init K");
}

int main(int argc, char **argv) {
  struct cl_config cfg;
  char err[128];
  hs_thread("main", 0);
  if (cl_config_parse(&cfg, argc, argv, err, sizeof err) != 0) {
    fprintf(stderr, "condlock: usage: %s\ncondlock: condlock [-n N] [-r R] [-tick-ms T] [-q Q]\n", err);
    hs_exit(CL_EXIT_USAGE);
  }

  struct cl_ctx ctx;
  init_ctx(&ctx, &cfg);

  struct cl_worker_arg args[CL_MAX_WORKERS];
  pthread_t workers[CL_MAX_WORKERS];
#ifndef CONDLOCK_NOWD
  pthread_t wd;
  cl_ok(pthread_create(&wd, NULL, cl_watchdog_main, &ctx), "create watchdog");
#endif
  for (unsigned i = 0; i < cfg.workers; i++) {
    args[i] = (struct cl_worker_arg){&ctx, i};
    cl_ok(pthread_create(&workers[i], NULL, cl_worker_main, &args[i]), "create worker");
  }
  for (unsigned i = 0; i < cfg.workers; i++) cl_ok(pthread_join(workers[i], NULL), "join worker");
#ifndef CONDLOCK_NOWD
  atomic_store_explicit(&ctx.stop, 1, RELAXED);
  cl_ok(pthread_join(wd, NULL), "join watchdog");
#endif

  const uint64_t done = atomic_load_explicit(&ctx.rounds_done, RELAXED);
  const uint64_t want = (uint64_t)cfg.rounds * cfg.workers;
  HS_ALWAYS(done == want, "condlock.harness.all_rounds_ran", "every worker ran every round", (int64_t)done,
            (int64_t)want);
  cl_print_report(&ctx);
  hs_exit(done == want ? 0 : CL_EXIT_HARNESS);
}
