/*
** C lost-update fixture. Build -DLANG_AT_LOCKED for the locked twin; without it, the racy program.
** Property: c.counter_exact (always): the final counter equals threads * iterations. Coverage: c.all_started.
** Output: one fixed-width line on stdout. Exit status: 0, whatever the property says.
*/
#include <pthread.h>
#include <stdint.h>

#include "hairspring.h"
#include "lang_at.h"

static uint64_t counter;
static unsigned started;
static unsigned arrivals;
static uint8_t order[LANG_AT_THREADS];

#ifdef LANG_AT_LOCKED
static pthread_mutex_t gate = PTHREAD_MUTEX_INITIALIZER;
#define ENTER() pthread_mutex_lock(&gate)
#define LEAVE() pthread_mutex_unlock(&gate)
#else
#define ENTER() ((void)0)
#define LEAVE() ((void)0)
#endif

static void *worker(void *arg) {
  unsigned id = (unsigned)(uintptr_t)arg;
  hs_thread("worker", id);
  __atomic_fetch_add(&started, 1u, __ATOMIC_RELAXED);
  for (unsigned i = 0; i < LANG_AT_ITERS; i++) {
    ENTER();
    uint64_t seen = __atomic_load_n(&counter, __ATOMIC_RELAXED);
    (void)lang_at_spin(seen, LANG_AT_SPIN); /* the window: user code only, no system call */
    __atomic_store_n(&counter, seen + 1u, __ATOMIC_RELAXED);
    LEAVE();
    HS_WORK(1);
  }
  order[__atomic_fetch_add(&arrivals, 1u, __ATOMIC_RELAXED)] = (uint8_t)id;
  return NULL;
}

int main(void) {
  pthread_t threads[LANG_AT_THREADS];
  for (unsigned i = 0; i < LANG_AT_THREADS; i++) pthread_create(&threads[i], NULL, worker, (void *)(uintptr_t)i);
  for (unsigned i = 0; i < LANG_AT_THREADS; i++) pthread_join(threads[i], NULL);

  uint64_t final_value = __atomic_load_n(&counter, __ATOMIC_RELAXED);
  HS_SOMETIMES(started == LANG_AT_THREADS, "c.all_started", "every worker started", (int64_t)started);
  HS_ALWAYS(final_value == LANG_AT_EXPECTED, "c.counter_exact", "no update was lost", (int64_t)final_value,
            (int64_t)LANG_AT_EXPECTED);
  lang_at_print("c", final_value, lang_at_order_hash(order, LANG_AT_THREADS), lang_at_random());
  return 0;
}
