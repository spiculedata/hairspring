/* The watchdog: the only thread that sleeps and the only one that must never block untimed.
 *
 * Every tick it reads `progress`. After Q ticks without progress (and with workers still unfinished) it looks at the
 * lock state under G (trylock-poll, never an untimed lock): the attributed rule decides HIT (exit 3); any other
 * quiescent state is a STALL (exit 4) with a subtype. The process is ended from here because a hit leaves the
 * workers blocked forever. The property site is evaluated at every tick so that healthy runs have evaluations. */
#include <stdio.h>
#include <string.h>

#include "cl_check.h"
#include "cl_lock.h"
#include "cl_report.h"
#include "cl_sdk.h"
#include "cl_worker.h"

#define RELAXED memory_order_relaxed
#define CL_EXIT_HIT 3
#define CL_EXIT_STALL 4

#define PROP_ID "glibc.cond.signal_wakes_one_of_blocked"
#define STALL_ID "condlock.stall_unexplained"

/* What one tick learned about the lock state. */
struct tick_view {
  int have_lock;
  int is_locked;
  int stuck;
  enum cl_subtype subtype;
  uint64_t s_seq;
  uint32_t blocked_n;
  uint32_t waiting_n;
  struct cl_condraw raw;
};

static uint32_t count_waiting(const struct cl_ctx *c) {
  uint32_t n = 0;
  for (unsigned i = 0; i < c->cfg.workers; i++) n += c->waiting[i] ? 1u : 0u;
  return n;
}

/* G is held. */
static void view_under_lock(const struct cl_ctx *c, struct tick_view *v) {
  v->have_lock = 1;
  v->is_locked = c->is_locked;
  v->stuck = cl_attributed_stuck(c->is_locked, &c->snap, c->waiting, c->returns, c->cfg.workers);
  v->subtype = cl_classify_stall(1, c->is_locked, c->waiting, c->cfg.workers);
  v->s_seq = c->snap.seq;
  v->blocked_n = c->snap.blocked_n;
  v->waiting_n = count_waiting(c);
  v->raw = cl_condraw_read(&c->l);
}

/* G could not be taken for a whole tick: describe the state from the atomic mirrors. */
static void view_without_lock(const struct cl_ctx *c, struct tick_view *v) {
  v->have_lock = 0;
  v->is_locked = atomic_load_explicit(&c->mirror_locked, RELAXED) != 0;
  v->stuck = 0;
  v->subtype = CL_SUB_M;
  v->s_seq = atomic_load_explicit(&c->st_signals, RELAXED);
  v->blocked_n = 0;
  v->waiting_n = (uint32_t)atomic_load_explicit(&c->mirror_nwait, RELAXED);
  memset(&v->raw, 0, sizeof v->raw);
  v->raw.arm = CL_ARM_NONE;
}

/* Prints the report line and the state dump, then ends the process (the workers are blocked for good). */
static void end_run(struct cl_ctx *c, const char *what, const struct tick_view *v, enum cl_subtype subtype,
                    uint64_t progress, uint32_t stagnant, int status) {
  struct cl_dump d = {0};
  char line[CL_LINE_MAX * 2];
  d.what = what;
  d.subtype = subtype;
  d.s_seq = v->s_seq;
  d.progress = progress;
  d.ticks = atomic_load_explicit(&c->st_ticks, RELAXED);
  d.blocked_n = v->blocked_n;
  d.stagnant = stagnant;
  d.waiting_n = v->waiting_n;
  d.is_locked = v->is_locked;
  d.raw = v->raw;
  cl_print_report(c);
  int n = cl_dump_format(line, sizeof line - 1, &d);
  if (n > 0) {
    line[n] = '\n';
    line[n + 1] = '\0';
    fputs(line, stderr);
  }
  hs_exit(status);
}

static void tick(struct cl_ctx *c, struct cl_stag *stag) {
  uint64_t progress = atomic_load_explicit(&c->progress, RELAXED);
  uint32_t stagnant = cl_stag_update(stag, progress);
  atomic_fetch_add_explicit(&c->st_ticks, 1, RELAXED);
  atomic_store_explicit(&c->st_max_stagnant, stag->max_stagnant, RELAXED);
  HS_REACHABLE("condlock.watchdog_tick", "the watchdog evaluated at least once");
  HS_SOMETIMES(stagnant >= 1, "condlock.stagnation_seen", "a tick without progress", (int64_t)stagnant);

  const int quiescent = stagnant >= c->cfg.quorum && atomic_load_explicit(&c->finished, RELAXED) < c->cfg.workers;
  struct tick_view v;
  memset(&v, 0, sizeof v);
  v.raw.arm = CL_ARM_NONE;
  if (quiescent) {
    if (cl_trylock_poll(&c->g, c->cfg.tick_ms)) {
      view_under_lock(c, &v);
      cl_ok(pthread_mutex_unlock(&c->g), "unlock G (watchdog)");
    } else {
      view_without_lock(c, &v);
    }
  }
  const int hit = quiescent && v.stuck;
  const enum cl_subtype witness = hit ? (enum cl_subtype)(v.subtype | CL_SUB_ATTRIBUTED) : v.subtype;
  HS_ALWAYS(!hit, PROP_ID, "a signal wakes one of the threads that were blocked when it was sent", (int64_t)v.s_seq,
            (int64_t)v.blocked_n, (int64_t)progress, (int64_t)witness);
  if (hit) end_run(c, "HIT", &v, witness, progress, stagnant, CL_EXIT_HIT);
  if (quiescent) {
    HS_UNREACHABLE(STALL_ID, "quiescent and not explained by the attributed rule", (int64_t)v.s_seq,
                   (int64_t)v.blocked_n, (int64_t)progress, (int64_t)v.subtype);
    end_run(c, "STALL", &v, v.subtype, progress, stagnant, CL_EXIT_STALL);
  }
}

void *cl_watchdog_main(void *arg) {
  struct cl_ctx *c = (struct cl_ctx *)arg;
  struct cl_stag stag;
  memset(&stag, 0, sizeof stag);
  hs_thread("wd", 0);
  for (;;) {
    cl_sleep_ms(c->cfg.tick_ms);
    if (atomic_load_explicit(&c->stop, RELAXED)) break;
    tick(c, &stag);
  }
  return NULL;
}
