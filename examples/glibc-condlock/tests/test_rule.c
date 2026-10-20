/* Unit tests for the stuck rule, the subtype classifier and the stagnation counter. */
#include <string.h>

#include "cl_rule.h"
#include "hs_test.h"

#define N 4

static void snap_of(struct cl_snapshot *s, const uint8_t *waiting, const uint64_t *returns) {
  *s = cl_snapshot_take(7, waiting, returns, N);
}

static void test_snapshot_records_blocked_set_and_returns(void) {
  uint8_t waiting[N] = {1, 0, 1, 0};
  uint64_t returns[N] = {3, 4, 5, 6};
  struct cl_snapshot s;
  snap_of(&s, waiting, returns);
  CHECK_U64(s.seq, 7);
  CHECK_U64(s.blocked_n, 2);
  CHECK(s.blocked[0] == 1 && s.blocked[1] == 0 && s.blocked[2] == 1 && s.blocked[3] == 0);
  CHECK_U64(s.ret_at[2], 5);
  CHECK_U64(s.ret_at[3], 6);
}

static void test_stuck_when_lock_free_and_all_blocked_still_blocked(void) {
  uint8_t waiting[N] = {1, 0, 1, 0};
  uint64_t returns[N] = {3, 4, 5, 6};
  struct cl_snapshot s;
  snap_of(&s, waiting, returns);
  CHECK(cl_attributed_stuck(0, &s, waiting, returns, N) == 1);
}

static void test_not_stuck_when_lock_is_held(void) {
  uint8_t waiting[N] = {1, 1, 0, 0};
  uint64_t returns[N] = {0};
  struct cl_snapshot s;
  snap_of(&s, waiting, returns);
  CHECK(cl_attributed_stuck(1, &s, waiting, returns, N) == 0);
}

static void test_not_stuck_when_snapshot_is_empty(void) {
  uint8_t waiting[N] = {0, 0, 0, 0};
  uint64_t returns[N] = {0};
  struct cl_snapshot s;
  snap_of(&s, waiting, returns);
  CHECK_U64(s.blocked_n, 0);
  CHECK(cl_attributed_stuck(0, &s, waiting, returns, N) == 0);
}

static void test_not_stuck_when_one_member_returned_and_blocked_again(void) {
  /* worker 2 was woken (returns went up) and is waiting again: it was not stuck in the same wait */
  uint8_t waiting[N] = {1, 0, 1, 0};
  uint64_t returns[N] = {3, 4, 5, 6};
  struct cl_snapshot s;
  snap_of(&s, waiting, returns);
  returns[2] = 6;
  CHECK(cl_attributed_stuck(0, &s, waiting, returns, N) == 0);
}

static void test_not_stuck_when_one_member_is_no_longer_waiting(void) {
  uint8_t waiting[N] = {1, 0, 1, 0};
  uint64_t returns[N] = {3, 4, 5, 6};
  struct cl_snapshot s;
  snap_of(&s, waiting, returns);
  waiting[0] = 0;
  CHECK(cl_attributed_stuck(0, &s, waiting, returns, N) == 0);
}

static void test_a_new_waiter_outside_the_snapshot_does_not_matter(void) {
  uint8_t waiting[N] = {1, 0, 0, 0};
  uint64_t returns[N] = {0};
  struct cl_snapshot s;
  snap_of(&s, waiting, returns);
  waiting[3] = 1;
  CHECK(cl_attributed_stuck(0, &s, waiting, returns, N) == 1);
}

static void test_classify_without_the_lock_is_subtype_m(void) {
  uint8_t waiting[N] = {1, 0, 0, 0};
  CHECK_U64(cl_classify_stall(0, 0, waiting, N), CL_SUB_M);
}

static void test_classify_lock_held_is_subtype_l_even_with_waiters(void) {
  uint8_t waiting[N] = {1, 1, 0, 0};
  CHECK_U64(cl_classify_stall(1, 1, waiting, N), CL_SUB_L);
}

static void test_classify_free_lock_with_waiter_is_subtype_q(void) {
  uint8_t waiting[N] = {0, 0, 1, 0};
  CHECK_U64(cl_classify_stall(1, 0, waiting, N), CL_SUB_Q);
}

static void test_classify_free_lock_without_waiter_is_subtype_b(void) {
  uint8_t waiting[N] = {0, 0, 0, 0};
  CHECK_U64(cl_classify_stall(1, 0, waiting, N), CL_SUB_B);
}

static void test_stag_counts_consecutive_unchanged_ticks_and_resets(void) {
  struct cl_stag s;
  memset(&s, 0, sizeof s);
  CHECK_U64(cl_stag_update(&s, 5), 0);  /* moved from 0 */
  CHECK_U64(cl_stag_update(&s, 5), 1);
  CHECK_U64(cl_stag_update(&s, 5), 2);
  CHECK_U64(cl_stag_update(&s, 9), 0);  /* progress resets */
  CHECK_U64(cl_stag_update(&s, 9), 1);
  CHECK_U64(s.max_stagnant, 2);
}

static void test_stag_first_tick_with_no_progress_counts(void) {
  struct cl_stag s;
  memset(&s, 0, sizeof s);
  CHECK_U64(cl_stag_update(&s, 0), 1);
  CHECK_U64(s.max_stagnant, 1);
}

int main(void) {
  RUN(test_snapshot_records_blocked_set_and_returns);
  RUN(test_stuck_when_lock_free_and_all_blocked_still_blocked);
  RUN(test_not_stuck_when_lock_is_held);
  RUN(test_not_stuck_when_snapshot_is_empty);
  RUN(test_not_stuck_when_one_member_returned_and_blocked_again);
  RUN(test_not_stuck_when_one_member_is_no_longer_waiting);
  RUN(test_a_new_waiter_outside_the_snapshot_does_not_matter);
  RUN(test_classify_without_the_lock_is_subtype_m);
  RUN(test_classify_lock_held_is_subtype_l_even_with_waiters);
  RUN(test_classify_free_lock_with_waiter_is_subtype_q);
  RUN(test_classify_free_lock_without_waiter_is_subtype_b);
  RUN(test_stag_counts_consecutive_unchanged_ticks_and_resets);
  RUN(test_stag_first_tick_with_no_progress_counts);
  return hs_t_summary();
}
