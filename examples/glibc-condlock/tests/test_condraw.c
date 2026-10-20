/* cl_condraw_read against a stand-in glibc pthread_cond_t (tests/stand-in/arm-u or arm-c, picked by -DCONDLOCK_ARM_U or
 * -DCONDLOCK_ARM_C together with -D__GLIBC__=2). The counters are __atomic_wide_counter unions: the 64-bit value must
 * come from __value64. */
#include <string.h>

#include "cl_report.h"
#include "hs_test.h"
#include <pthread.h>

static void test_reads_the_wide_counters_and_the_small_fields(void) {
  pthread_cond_t c;
  memset(&c, 0, sizeof c);
  c.__data.__wseq.__value64 = 0x100000009ull;
  c.__data.__g1_start.__value64 = 0x200000004ull;
  c.__data.__g_size[0] = 1;
  c.__data.__g_size[1] = 3;
  c.__data.__g1_orig_size = 5;
  c.__data.__wrefs = 3u << 3;
  c.__data.__g_signals[0] = 2;
  c.__data.__g_signals[1] = 7;
#ifdef CONDLOCK_ARM_U
  c.__data.__g_refs[0] = 4;
  c.__data.__g_refs[1] = 6;
#endif
  struct cl_condraw raw = cl_condraw_read(&c);
  CHECK_U64(raw.wseq, 0x100000009ull);
  CHECK_U64(raw.g1_start, 0x200000004ull);
  CHECK_U64(raw.g_size[0], 1);
  CHECK_U64(raw.g_size[1], 3);
  CHECK_U64(raw.g1_orig_size, 5);
  CHECK_U64(raw.wrefs, 3u << 3);
  CHECK_U64(raw.g_signals[0], 2);
  CHECK_U64(raw.g_signals[1], 7);
#ifdef CONDLOCK_ARM_U
  CHECK_U64(raw.arm, CL_ARM_U);
  CHECK_U64(raw.g_refs[0], 4);
  CHECK_U64(raw.g_refs[1], 6);
#else
  CHECK_U64(raw.arm, CL_ARM_C);
  CHECK_U64(raw.g_refs[0], 0);
#endif
}

static void test_wide_counter_is_not_truncated_to_32_bits(void) {
  pthread_cond_t c;
  memset(&c, 0, sizeof c);
  c.__data.__wseq.__value32.__high = 0xdeadbeefu;
  c.__data.__wseq.__value32.__low = 0x1u;
  struct cl_condraw raw = cl_condraw_read(&c);
  CHECK_U64(raw.wseq, 0xdeadbeef00000001ull); /* __low is the first member: little-endian hosts only (x86-64, arm64) */
}

int main(void) {
  RUN(test_reads_the_wide_counters_and_the_small_fields);
  RUN(test_wide_counter_is_not_truncated_to_32_bits);
  return hs_t_summary();
}
