/* cl_ctx.h -- the one shared state object of a run. It lives on main's stack; threads get a pointer. There are no
 * file-scope variables in the workload apart from the SDK's own call-site records.
 *
 * Locking: fields marked [G] are protected by the lock mutex `g`, fields marked [B] by the barrier mutex `b`.
 * Fields marked [A] are atomics (relaxed) that the watchdog and the final report read without a mutex. */
#ifndef CL_CTX_H
#define CL_CTX_H

#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>

#include "cl_config.h"
#include "cl_plant.h"
#include "cl_rule.h"

typedef _Atomic uint64_t cl_counter;

struct cl_ctx {
  struct cl_config cfg;

  pthread_mutex_t g; /* the lock's own mutex (G) */
  pthread_cond_t l;  /* the lock's condvar (L): the one under test */
  pthread_mutex_t b; /* barrier mutex (B) */
  pthread_cond_t k;  /* barrier condvar (K): separate from L on purpose */

  /* [G] the lock protocol */
  int is_locked;
  uint8_t waiting[CL_MAX_WORKERS];
  uint64_t returns[CL_MAX_WORKERS];
  uint64_t s_seq;           /* releases so far */
  struct cl_snapshot snap;  /* the latest release's snapshot */
  uint64_t plant_releases;  /* planted builds: releases counted toward k */
  int plant_deaf;           /* planted drop build: the condvar is deaf from now on */

  /* [B] the barrier */
  unsigned arrived;
  uint64_t round;
  uint64_t plant_arrivals;  /* planted barrier build: last arrivers counted toward k */

  /* [A] progress and mirrors the watchdog may read without G */
  cl_counter progress;      /* lock acquisitions + barrier arrivals */
  cl_counter finished;      /* workers that left their last round */
  cl_counter rounds_done;   /* worker-rounds completed */
  cl_counter holders;       /* harness: threads inside the critical section */
  cl_counter stop;          /* main asks the watchdog to leave */
  cl_counter plant_applied; /* planted build: the plant took effect */
  cl_counter mirror_locked; /* copy of is_locked, readable when G cannot be taken (stall subtype M) */
  cl_counter mirror_nwait;  /* copy of the number of threads blocked on L */

  /* [A] statistics for the stdout line */
  cl_counter st_signals, st_blocked1, st_max_blocked, st_spanned, st_ticks, st_max_stagnant;

  volatile uint64_t crit_counter; /* the "1000 increments" target; only touched inside the critical section */
};

#endif
