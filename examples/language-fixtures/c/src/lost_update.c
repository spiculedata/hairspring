/*
** C lost-update fixture: at-c buggy|fixed|wide (see ../../common/lang_at.h for the modes).
** Property: c.counter_exact (always): the final counter equals threads * iterations, witness values
** [final, 4000, 4, 1000]. Coverage: c.all_started (sometimes). Each is evaluated once, in main, after the join.
** Output: one contract line on stdout. Exit status: 0 whatever the property says, 3 for an internal invariant
** failure, 64 for a usage error.
*/
#include <pthread.h>
#include <sched.h>
#include <stdint.h>

#include "hairspring.h"
#include "lang_at.h"

static uint64_t counter;
static unsigned started;
static unsigned arrivals;
static unsigned abandoned; /* set when a worker could not be created, so the others stop waiting at the barrier */
static uint8_t order[LANG_AT_THREADS];
static pthread_mutex_t gate = PTHREAD_MUTEX_INITIALIZER;

struct worker_arg {
  unsigned id;
  const struct lang_at_mode *mode;
};

/* The start barrier: nobody iterates before everybody has started. Returns 0, or -1 if the run was abandoned. */
static int wait_for_all_workers(void) {
  while (__atomic_load_n(&started, __ATOMIC_ACQUIRE) < LANG_AT_THREADS) {
    if (__atomic_load_n(&abandoned, __ATOMIC_ACQUIRE)) return -1;
    sched_yield();
  }
  return 0;
}

static void *worker(void *raw) {
  const struct worker_arg *arg = raw;
  hs_thread("worker", arg->id);
  __atomic_fetch_add(&started, 1u, __ATOMIC_ACQ_REL);
  if (wait_for_all_workers() != 0) return NULL;
  for (unsigned i = 0; i < LANG_AT_ITERS; i++) {
    if (arg->mode->locked) pthread_mutex_lock(&gate);
    uint64_t seen = __atomic_load_n(&counter, __ATOMIC_RELAXED);
    uint64_t pad = lang_at_spin(seen, arg->mode->spin); /* the window: user code only, no system call */
    __atomic_store_n(&counter, seen + 1u, __ATOMIC_RELAXED);
    if (arg->mode->locked) pthread_mutex_unlock(&gate);
    lang_at_keep(arg->id, pad);
    HS_WORK(1);
  }
  order[__atomic_fetch_add(&arrivals, 1u, __ATOMIC_RELAXED)] = (uint8_t)('0' + arg->id);
  return NULL;
}

/* Starts the workers and joins the ones that started. Returns the number of workers that were created. */
static unsigned run_workers(const struct lang_at_mode *mode) {
  pthread_t threads[LANG_AT_THREADS];
  struct worker_arg args[LANG_AT_THREADS];
  unsigned created = 0;
  for (unsigned i = 0; i < LANG_AT_THREADS; i++) {
    args[i].id = i;
    args[i].mode = mode;
    if (pthread_create(&threads[i], NULL, worker, &args[i]) != 0) {
      __atomic_store_n(&abandoned, 1u, __ATOMIC_RELEASE);
      break;
    }
    created++;
  }
  for (unsigned i = 0; i < created; i++) pthread_join(threads[i], NULL);
  return created;
}

int main(int argc, char **argv) {
  const struct lang_at_mode *mode = lang_at_parse_mode(argc, argv);
  if (mode == NULL) return LANG_AT_EXIT_USAGE;

  unsigned created = run_workers(mode);
  if (created != LANG_AT_THREADS || arrivals != LANG_AT_THREADS) {
    fprintf(stderr, "at-c: %u workers created and %u finished, want %u\n", created, arrivals, LANG_AT_THREADS);
    return LANG_AT_EXIT_INVARIANT;
  }

  uint64_t final_value = __atomic_load_n(&counter, __ATOMIC_RELAXED);
  HS_ALWAYS(final_value == LANG_AT_EXPECTED, "c.counter_exact", "no update was lost", (int64_t)final_value,
            (int64_t)LANG_AT_EXPECTED, (int64_t)LANG_AT_THREADS, (int64_t)LANG_AT_ITERS);
  HS_SOMETIMES(started == LANG_AT_THREADS, "c.all_started", "every worker started", (int64_t)started);
  if (lang_at_print("c", mode->name, final_value, lang_at_order_hash(order, LANG_AT_THREADS), lang_at_random()) != 0) {
    fputs("at-c: cannot write the result line\n", stderr);
    return LANG_AT_EXIT_INVARIANT;
  }
  return 0;
}
