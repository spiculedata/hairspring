/* cl_rule.h -- the stuck rule as pure functions over plain data (no threads, no SDK), so it can be unit tested.
 *
 * A releaser holding the lock mutex G records a snapshot S of who was blocked in pthread_cond_wait when it sent a
 * signal. The watchdog calls a state "attributed stuck" when the lock is free, the snapshot is not empty and every
 * thread in it is still blocked in the same wait (it has not returned since the snapshot).
 */
#ifndef CL_RULE_H
#define CL_RULE_H

#include <stdint.h>

#define CL_MAX_WORKERS 16

/* The latest release, taken under G. */
struct cl_snapshot {
  uint64_t seq;                      /* S_seq: which release this is */
  uint32_t blocked_n;                /* |B(S)| */
  uint8_t blocked[CL_MAX_WORKERS];   /* blocked[i] = 1 when worker i was in pthread_cond_wait at S */
  uint64_t ret_at[CL_MAX_WORKERS];   /* returns[i] at S, for every i */
};

/* Subtype bits of a quiescent state (also the witness value reported with the property site). */
enum cl_subtype {
  CL_SUB_NONE = 0,
  CL_SUB_Q = 1,          /* lock free, at least one worker still blocked on the lock condvar */
  CL_SUB_L = 2,          /* lock held (leaked or held): nobody can make progress */
  CL_SUB_B = 4,          /* lock free, no lock waiter, barrier incomplete */
  CL_SUB_M = 8,          /* the lock mutex could not be acquired for a whole tick */
  CL_SUB_ATTRIBUTED = 16 /* added to the subtype of a property hit: the attributed rule held */
};

/* Fill a snapshot from the state under G. n <= CL_MAX_WORKERS. */
struct cl_snapshot cl_snapshot_take(uint64_t seq, const uint8_t *waiting, const uint64_t *returns, unsigned n);

/* The attributed rule: is_locked == 0, B non-empty, and for every i in B: waiting[i] == 1 and returns[i] ==
 * ret_at[i]. */
int cl_attributed_stuck(int is_locked, const struct cl_snapshot *snap, const uint8_t *waiting, const uint64_t *returns,
                        unsigned n);

/* Subtype of a quiescent state seen under G (have_lock = 1) or without it (have_lock = 0 gives CL_SUB_M). */
enum cl_subtype cl_classify_stall(int have_lock, int is_locked, const uint8_t *waiting, unsigned n);

/* Stagnation counter: consecutive ticks in which `progress` did not move. */
struct cl_stag {
  uint64_t last;
  uint32_t stagnant;
  uint32_t max_stagnant;
};

/* Feed one tick's progress value; returns the new stagnant count. The first tick compares against 0. */
uint32_t cl_stag_update(struct cl_stag *s, uint64_t progress);

#endif
