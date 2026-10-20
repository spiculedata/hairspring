#include "cl_lock.h"

#include "cl_check.h"
#include "cl_sdk.h"

#define RELAXED memory_order_relaxed
#define SPAN_SIGNALS 2u

static void note_max(cl_counter *max, uint64_t v) {
  uint64_t cur = atomic_load_explicit(max, RELAXED);
  while (v > cur && !atomic_compare_exchange_weak_explicit(max, &cur, v, RELAXED, RELAXED)) {
  }
}

/* Called with G held, right after pthread_cond_wait returned. */
static void note_wait_return(struct cl_ctx *c, unsigned id, uint64_t seq_at_wait) {
  c->waiting[id] = 0;
  c->returns[id]++;
  atomic_fetch_sub_explicit(&c->mirror_nwait, 1, RELAXED);
  uint64_t between = c->s_seq - seq_at_wait;
  HS_SOMETIMES(between >= SPAN_SIGNALS, "condlock.wait_spanned_two_signals",
               "a wait spanned at least two releases", (int64_t)between, (int64_t)id);
  if (between >= SPAN_SIGNALS) atomic_fetch_add_explicit(&c->st_spanned, 1, RELAXED);
}

/* Planted hold-G build: the k-th acquisition sleeps (a timer wait) while holding G. Called with G held. */
static void plant_holds_g(struct cl_ctx *c) {
#ifdef CONDLOCK_PLANT_HOLDG
  if (++c->plant_releases == (uint64_t)CONDLOCK_PLANT_HOLDG) {
    atomic_store_explicit(&c->plant_applied, 1, RELAXED);
    cl_sleep_ms(CL_HOLDG_SLEEP_MS);
  }
#else
  (void)c;
#endif
}

void cl_acquire(struct cl_ctx *c, unsigned id) {
  cl_ok(pthread_mutex_lock(&c->g), "lock G");
  plant_holds_g(c);
  while (c->is_locked) {
    uint64_t seq_at_wait = c->s_seq;
    c->waiting[id] = 1;
    atomic_fetch_add_explicit(&c->mirror_nwait, 1, RELAXED);
    cl_ok(pthread_cond_wait(&c->l, &c->g), "cond_wait L");
    note_wait_return(c, id, seq_at_wait);
  }
  c->is_locked = 1;
  atomic_store_explicit(&c->mirror_locked, 1, RELAXED);
  uint64_t before = atomic_fetch_add_explicit(&c->holders, 1, RELAXED);
  HS_ALWAYS(before == 0, "condlock.harness.lock_exclusive", "at most one thread inside the critical section",
            (int64_t)before, (int64_t)id);
  atomic_fetch_add_explicit(&c->progress, 1, RELAXED);
  cl_ok(pthread_mutex_unlock(&c->g), "unlock G");
}

/* Planted builds decide here; counted builds always return 0 (do not skip) and 1 (signal). */
static int plant_skips_unlock(struct cl_ctx *c) {
#ifdef CONDLOCK_PLANT_LEAK
  if (++c->plant_releases == (uint64_t)CONDLOCK_PLANT_LEAK) {
    atomic_store_explicit(&c->plant_applied, 1, RELAXED);
    return 1;
  }
#else
  (void)c;
#endif
  return 0;
}

static int plant_drops_signal(struct cl_ctx *c, const struct cl_snapshot *snap) {
#ifdef CONDLOCK_DROP_SIGNAL
  if (!c->plant_deaf && snap->blocked_n > 0 && ++c->plant_releases == (uint64_t)CONDLOCK_DROP_SIGNAL) {
    c->plant_deaf = 1;
    atomic_store_explicit(&c->plant_applied, 1, RELAXED);
  }
  return c->plant_deaf;
#else
  (void)c;
  (void)snap;
  return 0;
#endif
}

void cl_release(struct cl_ctx *c, unsigned id) {
  (void)id; /* used by the SDK build's coverage site only */
  cl_ok(pthread_mutex_lock(&c->g), "lock G");
  atomic_fetch_sub_explicit(&c->holders, 1, RELAXED);
  if (!plant_skips_unlock(c)) {
    c->is_locked = 0;
    atomic_store_explicit(&c->mirror_locked, 0, RELAXED);
  }
  c->s_seq++;
  c->snap = cl_snapshot_take(c->s_seq, c->waiting, c->returns, c->cfg.workers);
  atomic_fetch_add_explicit(&c->st_signals, 1, RELAXED);
  if (c->snap.blocked_n >= 1) atomic_fetch_add_explicit(&c->st_blocked1, 1, RELAXED);
  note_max(&c->st_max_blocked, c->snap.blocked_n);
  HS_SOMETIMES(c->snap.blocked_n >= 2, "condlock.contended_signal", "a release saw at least two blocked threads",
               (int64_t)c->snap.seq, (int64_t)c->snap.blocked_n, (int64_t)id);
  if (!plant_drops_signal(c, &c->snap)) cl_ok(pthread_cond_signal(&c->l), "cond_signal L");
  cl_ok(pthread_mutex_unlock(&c->g), "unlock G");
}

/* The last arriver of a round; planted barrier builds may skip the broadcast. Called with B held. */
static void finish_round(struct cl_ctx *c) {
  c->arrived = 0;
  c->round++;
#ifdef CONDLOCK_PLANT_BARRIER
  if (++c->plant_arrivals == (uint64_t)CONDLOCK_PLANT_BARRIER) {
    atomic_store_explicit(&c->plant_applied, 1, RELAXED);
    return;
  }
#endif
  cl_ok(pthread_cond_broadcast(&c->k), "cond_broadcast K");
}

void cl_barrier(struct cl_ctx *c, unsigned id) {
  (void)id;
  cl_ok(pthread_mutex_lock(&c->b), "lock B");
  atomic_fetch_add_explicit(&c->progress, 1, RELAXED);
  c->arrived++;
  if (c->arrived == c->cfg.workers) {
    finish_round(c);
  } else {
    uint64_t mine = c->round;
    while (c->round == mine) cl_ok(pthread_cond_wait(&c->k, &c->b), "cond_wait K");
  }
  cl_ok(pthread_mutex_unlock(&c->b), "unlock B");
}
