/* Unit tests for the stdout report line and the state dump line. */
#include <string.h>

#include "cl_report.h"
#include "hs_test.h"

static struct cl_report sample(void) {
  struct cl_report r;
  memset(&r, 0, sizeof r);
  r.libc = "none";
  r.hdr = "none";
  r.rounds_done = 4000;
  r.signals = 4000;
  r.signals_blocked = 3100;
  r.max_blocked = 7;
  r.ticks = 12;
  r.spanned = 55;
  r.max_stagnant = 2;
  return r;
}

static void test_report_line_exact_text(void) {
  char buf[CL_LINE_MAX];
  struct cl_report r = sample();
  int n = cl_report_format(buf, sizeof buf, &r);
  CHECK(n > 0);
  CHECK_STR(buf,
            "condlock libc=none hdr=none rounds=    4000 signals=    4000 blocked1=    3100 maxb=       7 "
            "maxstag=       2 ticks=      12 spanned=      55");
}

static void test_report_length_does_not_depend_on_counter_values(void) {
  char a[CL_LINE_MAX], b[CL_LINE_MAX];
  struct cl_report r1 = sample(), r2 = sample();
  r2.rounds_done = 1;
  r2.ticks = 99999999;
  r2.max_stagnant = 0;
  CHECK(cl_report_format(a, sizeof a, &r1) == cl_report_format(b, sizeof b, &r2));
}

static void test_report_applied_flag_only_in_planted_builds(void) {
  char buf[CL_LINE_MAX];
  struct cl_report r = sample();
  r.has_applied = 1;
  r.applied = 1;
  CHECK(cl_report_format(buf, sizeof buf, &r) > 0);
  CHECK(strstr(buf, " applied=1") != NULL);
  r.has_applied = 0;
  CHECK(cl_report_format(buf, sizeof buf, &r) > 0);
  CHECK(strstr(buf, "applied") == NULL);
}

static void test_report_rejects_a_buffer_that_is_too_small(void) {
  char buf[16];
  struct cl_report r = sample();
  CHECK(cl_report_format(buf, sizeof buf, &r) == -1);
}

static struct cl_dump dump_sample(enum cl_arm arm) {
  struct cl_dump d;
  memset(&d, 0, sizeof d);
  d.what = "STALL";
  d.subtype = CL_SUB_Q;
  d.s_seq = 41;
  d.progress = 900;
  d.ticks = 30;
  d.blocked_n = 2;
  d.stagnant = 10;
  d.waiting_n = 3;
  d.is_locked = 0;
  d.raw.arm = arm;
  return d;
}

static void test_dump_without_glibc_layout(void) {
  char buf[CL_LINE_MAX * 2];
  struct cl_dump d = dump_sample(CL_ARM_NONE);
  CHECK(cl_dump_format(buf, sizeof buf, &d) > 0);
  CHECK(strncmp(buf, "condlock: STALL ", 16) == 0);
  CHECK(strstr(buf, "subtype=Q") != NULL);
  CHECK(strstr(buf, "s_seq=41") != NULL);
  CHECK(strstr(buf, "waiting=3") != NULL);
  CHECK(strstr(buf, "cond=none") != NULL);
  CHECK(strchr(buf, '\n') == NULL);
}

static void test_dump_arm_u_prints_refs_and_the_group_cross_check(void) {
  char buf[CL_LINE_MAX * 2];
  struct cl_dump d = dump_sample(CL_ARM_U);
  d.raw.wseq = 9; /* low bit = G2 index 1, so G1 = 0 */
  d.raw.g1_start = 4;
  d.raw.g_refs[0] = 0;
  d.raw.g_refs[1] = 2;
  d.raw.g_size[0] = 1;
  d.raw.g_size[1] = 3;
  d.raw.g1_orig_size = 5;
  d.raw.wrefs = 3u << 3;
  d.raw.g_signals[0] = 2;
  d.raw.g_signals[1] = 0;
  CHECK(cl_dump_format(buf, sizeof buf, &d) > 0);
  CHECK(strstr(buf, "cond=U") != NULL);
  CHECK(strstr(buf, "g_refs=0,2") != NULL);
  CHECK(strstr(buf, "g_size=1,3") != NULL);
  CHECK(strstr(buf, "g_signals=2,0") != NULL);
  CHECK(strstr(buf, "g1=0") != NULL);
  CHECK(strstr(buf, "wrefs>>3=3") != NULL);
  CHECK(strstr(buf, "waiting=3") != NULL);
}

static void test_dump_arm_c_has_no_refs(void) {
  char buf[CL_LINE_MAX * 2];
  struct cl_dump d = dump_sample(CL_ARM_C);
  d.raw.wseq = 8;
  CHECK(cl_dump_format(buf, sizeof buf, &d) > 0);
  CHECK(strstr(buf, "cond=C") != NULL);
  CHECK(strstr(buf, "g_refs") == NULL);
  CHECK(strstr(buf, "g1=1") != NULL);
}

static void test_dump_hit_names_the_attributed_subtype(void) {
  char buf[CL_LINE_MAX * 2];
  struct cl_dump d = dump_sample(CL_ARM_NONE);
  d.what = "HIT";
  d.subtype = (enum cl_subtype)(CL_SUB_Q | CL_SUB_ATTRIBUTED);
  CHECK(cl_dump_format(buf, sizeof buf, &d) > 0);
  CHECK(strncmp(buf, "condlock: HIT ", 14) == 0);
  CHECK(strstr(buf, "subtype=Q+attributed") != NULL);
}

static void test_condraw_read_is_none_off_glibc(void) {
  int dummy = 0;
  struct cl_condraw raw = cl_condraw_read(&dummy);
#if defined(__GLIBC__) && (defined(CONDLOCK_ARM_U) || defined(CONDLOCK_ARM_C))
  (void)raw;
#else
  CHECK_U64(raw.arm, CL_ARM_NONE);
#endif
}

int main(void) {
  RUN(test_report_line_exact_text);
  RUN(test_report_length_does_not_depend_on_counter_values);
  RUN(test_report_applied_flag_only_in_planted_builds);
  RUN(test_report_rejects_a_buffer_that_is_too_small);
  RUN(test_dump_without_glibc_layout);
  RUN(test_dump_arm_u_prints_refs_and_the_group_cross_check);
  RUN(test_dump_arm_c_has_no_refs);
  RUN(test_dump_hit_names_the_attributed_subtype);
  RUN(test_condraw_read_is_none_off_glibc);
  return hs_t_summary();
}
