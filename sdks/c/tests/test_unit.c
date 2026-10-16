/* test_unit.c -- in-process unit tests: formatting, counters, witnesses, merging, thread keys, slots, lines. */
#include <pthread.h>
#include <sched.h>
#include <signal.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "../src/hs_internal.h"
#include "hs_test.h"

static const struct hs_site *find_site(const char *id) {
  for (const struct hs_site *s = hs_sites_begin(); s < hs_sites_end(); s++)
    if (hs_site_live(s) && strcmp(s->id, id) == 0) return s;
  return NULL;
}

static void fmt_to(struct hs_buf *b, char *store, size_t cap) { hs_buf_init(b, store, cap); }

/* ---- formatting ----------------------------------------------------------------------------------------------- */

static void test_numbers_are_fixed_width(void) {
  const uint64_t samples[] = {0, 7, 12345, UINT64_MAX};
  for (size_t i = 0; i < sizeof samples / sizeof samples[0]; i++) {
    char store[64];
    struct hs_buf b;
    fmt_to(&b, store, sizeof store);
    hs_put_u64_w(&b, samples[i]);
    CHECK_U64(b.len, HS_NUM_WIDTH);
  }
  char store[64];
  struct hs_buf b;
  fmt_to(&b, store, sizeof store);
  hs_put_i64_w(&b, INT64_MIN);
  CHECK_U64(b.len, HS_NUM_WIDTH);
  CHECK(memcmp(store, "-9223372036854775808", 20) == 0);
  fmt_to(&b, store, sizeof store);
  hs_put_i64_w(&b, -5);
  CHECK(memcmp(store, "                  -5", 20) == 0);
  fmt_to(&b, store, sizeof store);
  hs_put_u64_w(&b, UINT64_MAX);
  CHECK(memcmp(store, "18446744073709551615", 20) == 0);
}

static void test_text_is_escaped_and_ascii_only(void) {
  char store[128];
  struct hs_buf b;
  fmt_to(&b, store, sizeof store);
  hs_put_json_text(&b, "a\"b\\c\n\xc3\xa9z", 100);
  store[b.len] = 0;
  CHECK_STR(store, "\"a\\\"b\\\\c???z\"");
  fmt_to(&b, store, sizeof store);
  hs_put_json_text(&b, "abcdef", 3);
  store[b.len] = 0;
  CHECK_STR(store, "\"abc\"");
}

static void test_ident_replacement_and_equality(void) {
  char store[64];
  struct hs_buf b;
  fmt_to(&b, store, sizeof store);
  hs_put_ident(&b, "ok.id:1/x-y bad$", 128);
  store[b.len] = 0;
  CHECK_STR(store, "\"ok.id:1/x-y_bad_\"");
  CHECK(hs_ident_equal("a b", "a$b", 128));
  CHECK(!hs_ident_equal("ab", "abc", 128));
  CHECK(hs_ident_hash("a b", 128) == hs_ident_hash("a_b", 128));
}

static void test_buffer_overflow_is_sticky(void) {
  char store[4];
  struct hs_buf b;
  fmt_to(&b, store, sizeof store);
  hs_put_str(&b, "abcdef");
  CHECK(b.over);
  CHECK_U64(b.len, 4);
}

/* ---- counters and witnesses ----------------------------------------------------------------------------------- */

static int64_t g_val;

static void eval_always(int ok) { HS_ALWAYS(ok, "unit.always", "always msg", g_val, -22); }

static void test_counters_and_first_witnesses(void) {
  uint64_t base = hs__slot()->evals;
  uint64_t slot = hs__slot()->index;
  const int pattern[] = {1, 1, 1, 0, 0};
  for (size_t i = 0; i < 5; i++) {
    g_val = (int64_t)(100 * (i + 1));
    eval_always(pattern[i]);
  }
  const struct hs_site *s = find_site("unit.always");
  CHECK(s != NULL);
  CHECK_U64(s->trues, 3);
  CHECK_U64(s->falses, 2);
  CHECK_U64(s->first_true.ord, base + 1);
  CHECK_U64(s->first_true.slot, slot);
  CHECK_U64(s->first_true.vals[0], 100);
  CHECK(s->first_true.vals[1] == -22);
  CHECK_U64(s->first_false.ord, base + 4);
  CHECK_U64(s->first_false.vals[0], 400); /* the later false (500) did not overwrite it */
  CHECK_U64(hs__slot()->evals, base + 5);
}

static void test_point_kinds_count_hits_as_trues(void) {
  for (int i = 0; i < 4; i++) HS_REACHABLE("unit.reach", "reached", 1, 2, 3, 4);
  HS_UNREACHABLE("unit.unreach", "should not happen");
  const struct hs_site *r = find_site("unit.reach");
  CHECK_U64(r->trues, 4);
  CHECK_U64(r->falses, 0);
  CHECK_U64(r->first_false.ord, 0);
  CHECK_U64(r->first_true.vals[3], 4);
  CHECK_U64(r->kind, HS_KIND_REACHABLE);
  CHECK_U64(find_site("unit.unreach")->trues, 1);
  CHECK_U64(find_site("unit.unreach")->kind, HS_KIND_UNREACHABLE);
}

static void test_sometimes_and_always_or_unreachable_kinds(void) {
  HS_SOMETIMES(0, "unit.some", "some");
  HS_SOMETIMES(1, "unit.some", "some");
  HS_ALWAYS_OR_UNREACHABLE(1, "unit.aou", "aou");
  CHECK_U64(find_site("unit.some")->kind, HS_KIND_SOMETIMES);
  CHECK_U64(find_site("unit.aou")->kind, HS_KIND_ALWAYS_OR_UNREACHABLE);
}

static void test_work_accumulates_per_thread(void) {
  uint64_t before = hs__slot()->work;
  HS_WORK(5);
  HS_WORK(7);
  CHECK_U64(hs__slot()->work, before + 12);
}

/* ---- merging call sites --------------------------------------------------------------------------------------- */

static void __attribute__((noinline)) merge_site_a(int ok) { HS_SOMETIMES(ok, "unit.merge", "shared", 1); }
static void __attribute__((noinline)) merge_site_b(int ok) { HS_SOMETIMES(ok, "unit.merge", "shared", 2); }

static void test_same_id_at_two_sites_merges(void) {
  merge_site_a(1);
  merge_site_a(0);
  merge_site_b(1);
  merge_site_b(1);
  const struct hs_site *first = NULL, *second = NULL;
  for (const struct hs_site *s = hs_sites_begin(); s < hs_sites_end(); s++) {
    if (!hs_site_live(s) || strcmp(s->id, "unit.merge") != 0) continue;
    if (!first) first = s;
    else second = s;
  }
  CHECK(first != NULL && second != NULL);
  CHECK(first->canon == first);
  CHECK(second->canon == first);
  hs_catalog_sum();
  CHECK_U64(first->sum_trues, 3);
  CHECK_U64(first->sum_falses, 1);
}

static void test_distinct_id_count_matches_catalog(void) {
  uint64_t distinct = 0;
  for (const struct hs_site *s = hs_sites_begin(); s < hs_sites_end(); s++) {
    if (!hs_site_live(s)) continue;
    int seen = 0;
    for (const struct hs_site *t = hs_sites_begin(); t < s; t++)
      if (hs_site_live(t) && strcmp(t->id, s->id) == 0) seen = 1;
    if (!seen) distinct++;
  }
  CHECK_U64(hs_proc.n_ids, distinct);
}

/* ---- threads and slots ---------------------------------------------------------------------------------------- */

struct thread_job {
  const char *label;
  unsigned index;
  int result;
  char key[72];
  uint64_t stable;
};

static void *thread_job_main(void *arg) {
  struct thread_job *j = (struct thread_job *)arg;
  j->result = hs_thread(j->label, j->index);
  struct hs_slot *sl = hs__slot();
  memcpy(j->key, sl->key, sizeof j->key); /* both are char[72] */
  j->stable = sl->is_stable;
  return NULL;
}

static void run_job(struct thread_job *j) {
  pthread_t t;
  CHECK(pthread_create(&t, NULL, thread_job_main, j) == 0);
  pthread_join(t, NULL);
}

static void test_thread_keys(void) {
  struct thread_job a = {"writer", 3, -9, {0}, 0};
  run_job(&a);
  CHECK(a.result == 0);
  CHECK_STR(a.key, "writer-3");
  CHECK_U64(a.stable, 1);

  struct thread_job dup = {"writer", 3, -9, {0}, 0};
  run_job(&dup);
  CHECK(dup.result == -1);
  CHECK(strncmp(dup.key, "t#", 2) == 0);
  CHECK_U64(dup.stable, 0);

  struct thread_job odd = {"a b$", 0, -9, {0}, 0};
  run_job(&odd);
  CHECK(odd.result == 0);
  CHECK_STR(odd.key, "a_b_-0");

  struct thread_job empty = {"", 0, -9, {0}, 0};
  run_job(&empty);
  CHECK(empty.result == -1);

  struct thread_job null_label = {NULL, 0, -9, {0}, 0};
  run_job(&null_label);
  CHECK(null_label.result == -1);

  char longlabel[200];
  memset(longlabel, 'x', sizeof longlabel - 1);
  longlabel[sizeof longlabel - 1] = 0;
  struct thread_job big = {longlabel, 0, -9, {0}, 0};
  run_job(&big);
  CHECK(big.result == -1);
}

static void *work_once(void *arg) {
  (void)arg;
  HS_WORK(1);
  return NULL;
}

static void test_more_threads_than_slots_share_the_overflow_slot(void) {
  for (int i = 0; i < 300; i++) {
    pthread_t t;
    CHECK(pthread_create(&t, NULL, work_once, NULL) == 0);
    pthread_join(t, NULL);
  }
  CHECK_U64(hs_slots_used(), HS_MAX_THREADS);
  CHECK(hs_claims > HS_MAX_THREADS);
  CHECK_U64(hs_proc.dropped, hs_claims - HS_OVERFLOW_SLOT);
  CHECK_STR(hs_slots[HS_OVERFLOW_SLOT].key, "t#overflow");
  CHECK_U64(hs_slots[HS_OVERFLOW_SLOT].is_stable, 0);
}

/* ---- concurrency ---------------------------------------------------------------------------------------------- */

#define STRESS_THREADS 8
#define STRESS_ROUNDS 100000

static int g_go;

/* Start gate. Waiters yield so that they are less likely to keep the thread that is still creating threads off the CPU. */
static void wait_for_go(void) {
  while (!__atomic_load_n(&g_go, __ATOMIC_ACQUIRE)) sched_yield();
}

static void stress_hit(int ok) {
  struct hs_slot *sl = hs__slot();
  HS_SOMETIMES(ok, "unit.stress", "stress", (int64_t)sl->index, (int64_t)sl->evals + 1);
}

static void *stress_main(void *arg) {
  (void)arg;
  wait_for_go();
  for (int i = 0; i < STRESS_ROUNDS; i++) stress_hit(i & 1);
  return NULL;
}

static void test_contended_counters_are_exact_and_one_thread_owns_each_witness(void) {
  pthread_t t[STRESS_THREADS];
  for (int i = 0; i < STRESS_THREADS; i++) CHECK(pthread_create(&t[i], NULL, stress_main, NULL) == 0);
  __atomic_store_n(&g_go, 1, __ATOMIC_RELEASE);
  for (int i = 0; i < STRESS_THREADS; i++) pthread_join(t[i], NULL);
  const struct hs_site *s = find_site("unit.stress");
  CHECK_U64(s->trues, (uint64_t)STRESS_THREADS * STRESS_ROUNDS / 2);
  CHECK_U64(s->falses, (uint64_t)STRESS_THREADS * STRESS_ROUNDS / 2);
  /* A torn witness would mix one thread's slot with another's ordinal. */
  CHECK_U64(s->first_true.slot, (uint64_t)s->first_true.vals[0]);
  CHECK_U64(s->first_true.ord, (uint64_t)s->first_true.vals[1]);
  CHECK_U64(s->first_false.slot, (uint64_t)s->first_false.vals[0]);
  CHECK_U64(s->first_false.ord, (uint64_t)s->first_false.vals[1]);
  CHECK(s->first_true.ord >= 1 && s->first_false.ord >= 1);
}

#define CLAIMERS 48
static uint64_t g_claimed[CLAIMERS];

static void *claim_main(void *arg) {
  uint64_t i = (uint64_t)(uintptr_t)arg;
  wait_for_go();
  g_claimed[i] = hs__slot()->index;
  return NULL;
}

static void test_simultaneous_first_calls_get_distinct_slots(void) {
  __atomic_store_n(&g_go, 0, __ATOMIC_RELEASE);
  pthread_t t[CLAIMERS];
  for (uint64_t i = 0; i < CLAIMERS; i++) CHECK(pthread_create(&t[i], NULL, claim_main, (void *)(uintptr_t)i) == 0);
  __atomic_store_n(&g_go, 1, __ATOMIC_RELEASE);
  for (int i = 0; i < CLAIMERS; i++) pthread_join(t[i], NULL);
  for (int i = 0; i < CLAIMERS; i++)
    for (int j = i + 1; j < CLAIMERS; j++) CHECK(g_claimed[i] != g_claimed[j]);
}

/* ---- merging details ------------------------------------------------------------------------------------------ */

static void __attribute__((noinline)) order_site_a(int ok) { HS_SOMETIMES(ok, "unit.order", "ordering", 1); }
static void __attribute__((noinline)) order_site_b(int ok) { HS_SOMETIMES(ok, "unit.order", "ordering", 2); }

static void test_merged_witness_is_the_earliest_on_the_same_thread(void) {
  order_site_b(1); /* earlier ordinal, whichever record happens to be canonical */
  order_site_a(1);
  hs_catalog_sum();
  const struct hs_site *canon = NULL;
  uint64_t earliest = UINT64_MAX;
  for (const struct hs_site *s = hs_sites_begin(); s < hs_sites_end(); s++) {
    if (!hs_site_live(s) || strcmp(s->id, "unit.order") != 0) continue;
    if (s->canon == s) canon = s;
    if (s->first_true.ord && s->first_true.ord < earliest) earliest = s->first_true.ord;
  }
  CHECK(canon && canon->pick_true);
  CHECK_U64(canon->pick_true->ord, earliest);
}

static void conflict_always(void) { HS_ALWAYS(1, "unit.conflict", "as always"); }
static void conflict_sometimes(void) { HS_SOMETIMES(1, "unit.conflict", "as sometimes"); }

static size_t g_conflict_lines;

static void count_conflict_lines(const char *line, size_t len) {
  (void)len;
  if (strstr(line, "\"kind\":\"catalog\"") && strstr(line, "\"id\":\"unit.conflict\"")) g_conflict_lines++;
}

static void test_conflicting_kinds_keep_their_own_counts_and_both_get_a_catalog_line(void) {
  conflict_always();
  conflict_sometimes();
  conflict_sometimes();
  hs_catalog_sum();
  const struct hs_site *canon = NULL;
  for (const struct hs_site *s = hs_sites_begin(); s < hs_sites_end(); s++)
    if (hs_site_live(s) && strcmp(s->id, "unit.conflict") == 0 && s->canon == s) canon = s;
  CHECK(canon != NULL);
  CHECK_U64(canon->sum_trues, canon->trues); /* the other kind's hits are not folded in */
  g_conflict_lines = 0;
  hs_test_set_sink(count_conflict_lines);
  hs_emit_startup();
  hs_test_set_sink(NULL);
  CHECK_U64(g_conflict_lines, 2);
}

/* ---- the write path ------------------------------------------------------------------------------------------- */

#include <errno.h>

static void test_a_failed_write_keeps_errno_and_is_counted(void) {
  int saved_fd = hs_proc.fd;
  uint64_t dropped = hs_proc.dropped;
  hs_proc.fd = 9999; /* not an open descriptor */
  char store[64];
  struct hs_buf b;
  hs_buf_init(&b, store, sizeof store);
  hs_put_str(&b, "HAIRSPRING_EVENT {}\n");
  errno = 1234;
  hs_emit_line(&b);
  CHECK_U64(errno, 1234);
  CHECK_U64(hs_proc.dropped, dropped + 1);
  hs_proc.fd = saved_fd;
  hs_proc.dropped = dropped;
}

static void test_hs_report_leaves_errno_alone_even_when_the_events_descriptor_is_gone(void) {
  int saved_fd = hs_proc.fd;
  uint64_t dropped = hs_proc.dropped;
  hs_proc.fd = 9999; /* not open: the descriptor check's fstat fails with EBADF */
  hs_report_rearm();
  errno = 1234;
  hs_report();
  CHECK_U64(errno, 1234);
  hs_proc.fd = saved_fd;
  hs_proc.dropped = dropped;
  hs_report_rearm();
}

static void handler_for_test(int sig) { (void)sig; }

static void test_existing_signal_handlers_are_left_alone(void) {
  static const int signals[] = {SIGSEGV, SIGBUS, SIGABRT, SIGFPE, SIGILL, SIGTERM};
  struct sigaction saved[sizeof signals / sizeof signals[0]];
  stack_t saved_stack;
  sigaltstack(NULL, &saved_stack);
  for (size_t i = 0; i < sizeof signals / sizeof signals[0]; i++) sigaction(signals[i], NULL, &saved[i]);

  struct sigaction mine;
  memset(&mine, 0, sizeof mine);
  mine.sa_handler = handler_for_test;
  sigemptyset(&mine.sa_mask);
  sigaction(SIGTERM, &mine, NULL);
  CHECK(hs_install_signal_handlers() > 0); /* counted like a failed install */
  struct sigaction now;
  sigaction(SIGTERM, NULL, &now);
  CHECK(now.sa_handler == handler_for_test);

  for (size_t i = 0; i < sizeof signals / sizeof signals[0]; i++) sigaction(signals[i], &saved[i], NULL);
  sigaltstack(&saved_stack, NULL);
}

static void test_ids_and_keys_are_cut_at_their_limits(void) {
  char longid[300];
  memset(longid, 'a', sizeof longid - 1);
  longid[sizeof longid - 1] = 0;
  char store[400];
  struct hs_buf b;
  hs_buf_init(&b, store, sizeof store);
  hs_put_ident(&b, longid, HS_ID_MAX);
  CHECK_U64(b.len, HS_ID_MAX + 2); /* the quotes */
  hs_buf_init(&b, store, sizeof store);
  hs_put_thread_key(&b, longid, HS_THREAD_KEY_MAX);
  CHECK_U64(b.len, HS_THREAD_KEY_MAX + 2);
}

/* ---- lines ---------------------------------------------------------------------------------------------------- */

/* The offline section is a run of separate string globals. Under AddressSanitizer each has a poisoned redzone around
** it, so the walk below must not be instrumented and must not call libc string functions (their interceptors check
** the shadow). Redzone padding is zero bytes, so it reads as empty strings and is skipped. A future extractor that
** reads this section from the binary file never sees redzones, but must skip NUL runs the same way. */
#if defined(__clang__) || (defined(__GNUC__) && __GNUC__ >= 8)
#define HS_T_NO_ASAN __attribute__((no_sanitize("address")))
#else
#define HS_T_NO_ASAN
#endif

HS_T_NO_ASAN static int offline_section_has(const char *want) {
  const char *p = hs_offline_begin();
  const char *end = hs_offline_end();
  while (p < end) {
    if (*p == 0) {
      p++;
      continue;
    }
    const char *a = p, *b = want;
    while (*a && *a == *b) {
      a++;
      b++;
    }
    if (*a == 0 && *b == 0) return 1;
    while (*a) a++;
    p = a + 1;
  }
  return 0;
}

static void test_catalog_line_matches_offline_twin(void) {
  const struct hs_site *s = find_site("unit.always");
  char store[HS_LINE_MAX];
  struct hs_buf b;
  hs_buf_init(&b, store, sizeof store);
  CHECK(hs_line_catalog(&b, s) == 0);
  store[b.len] = 0;
  CHECK(strncmp(store, "HAIRSPRING_EVENT {\"v\":1,\"kind\":\"catalog\",\"proc\":\"", 49) == 0);
  CHECK(store[b.len - 1] == '\n');
  CHECK(strstr(store, "\"must_hit\":true") != NULL);

  /* Remove the proc member and compare with the twin that sits in the offline section. */
  char needle[HS_TAG_MAX + 16];
  size_t n = (size_t)snprintf(needle, sizeof needle, "\"proc\":\"%s\",", hs_proc.tag);
  char *at = strstr(store, needle);
  CHECK(at != NULL);
  if (at) memmove(at, at + n, strlen(at + n) + 1);
  size_t body = strlen("HAIRSPRING_EVENT ");
  store[strlen(store) - 1] = 0; /* newline */
  int found = offline_section_has(store + body);
  CHECK(found);
}

static void test_assert_line_length_ignores_counter_values(void) {
  const struct hs_site *s = find_site("unit.always");
  struct hs_site copy = *s;
  char store[HS_LINE_MAX];
  struct hs_buf b;
  size_t lens[4];
  struct hs_site variants[4];
  for (int i = 0; i < 4; i++) variants[i] = copy;
  variants[0].trues = 0, variants[0].falses = 0, variants[0].sum_trues = 0, variants[0].sum_falses = 0;
  variants[1].sum_trues = 1, variants[1].sum_falses = 0;
  variants[2].sum_trues = UINT64_MAX / 2, variants[2].sum_falses = UINT64_MAX / 2 - 1;
  variants[3].sum_trues = 0, variants[3].sum_falses = 9;
  for (int i = 0; i < 4; i++) {
    variants[i].canon = &variants[i];
    variants[i].pick_true = i == 1 ? &copy.first_true : NULL;
    variants[i].pick_false = i >= 2 ? &copy.first_false : NULL;
    hs_buf_init(&b, store, sizeof store);
    CHECK(hs_line_assert(&b, &variants[i]) == 0);
    lens[i] = b.len;
  }
  CHECK_U64(lens[1], lens[0]);
  CHECK_U64(lens[2], lens[0]);
  CHECK_U64(lens[3], lens[0]);
}

static char captured[512][HS_LINE_MAX + 1];
static size_t captured_len[512];
static size_t captured_n;

static void capture(const char *line, size_t len) {
  if (captured_n >= 512 || len > HS_LINE_MAX) return;
  memcpy(captured[captured_n], line, len);
  captured[captured_n][len] = 0;
  captured_len[captured_n++] = len;
}

static size_t count_kind(const char *kind) {
  char needle[32];
  snprintf(needle, sizeof needle, "\"kind\":\"%s\"", kind);
  size_t n = 0;
  for (size_t i = 0; i < captured_n; i++)
    if (strstr(captured[i], needle)) n++;
  return n;
}

static void test_report_has_one_assert_per_id_and_bounded_lines(void) {
  hs_test_set_sink(capture);
  hs_report_now(HS_END_EXPLICIT, 0);
  hs_test_set_sink(NULL);
  CHECK(captured_n > 0);
  for (size_t i = 0; i < captured_n; i++) {
    CHECK(captured_len[i] <= HS_LINE_MAX);
    CHECK(strncmp(captured[i], "HAIRSPRING_EVENT {", 18) == 0);
    CHECK(captured[i][captured_len[i] - 1] == '\n');
  }
  CHECK_U64(count_kind("assert"), hs_proc.n_ids);
  CHECK_U64(count_kind("end"), 1);
  CHECK_U64(count_kind("work"), 1);
  CHECK_U64(count_kind("thread"), hs_slots_used());
  char want[64];
  snprintf(want, sizeof want, "\"sites\":%llu,", (unsigned long long)hs_proc.n_ids);
  CHECK(strstr(captured[captured_n - 1], want) != NULL);
  snprintf(want, sizeof want, "\"threads\":%llu}", (unsigned long long)hs_slots_used());
  CHECK(strstr(captured[captured_n - 1], want) != NULL);
  CHECK(strstr(captured[captured_n - 1], "\"reason\":\"explicit\"") != NULL);
  /* The merged id appears once. */
  size_t merged = 0;
  for (size_t i = 0; i < captured_n; i++)
    if (strstr(captured[i], "\"kind\":\"assert\"") && strstr(captured[i], "\"id\":\"unit.merge\"")) merged++;
  CHECK_U64(merged, 1);
}

int main(void) {
  RUN(test_numbers_are_fixed_width);
  RUN(test_text_is_escaped_and_ascii_only);
  RUN(test_ident_replacement_and_equality);
  RUN(test_buffer_overflow_is_sticky);
  RUN(test_counters_and_first_witnesses);
  RUN(test_point_kinds_count_hits_as_trues);
  RUN(test_sometimes_and_always_or_unreachable_kinds);
  RUN(test_work_accumulates_per_thread);
  /* Before the overflow test: these need threads that own their slots. */
  RUN(test_contended_counters_are_exact_and_one_thread_owns_each_witness);
  RUN(test_simultaneous_first_calls_get_distinct_slots);
  RUN(test_same_id_at_two_sites_merges);
  RUN(test_distinct_id_count_matches_catalog);
  RUN(test_thread_keys);
  RUN(test_more_threads_than_slots_share_the_overflow_slot);
  RUN(test_merged_witness_is_the_earliest_on_the_same_thread);
  RUN(test_conflicting_kinds_keep_their_own_counts_and_both_get_a_catalog_line);
  RUN(test_a_failed_write_keeps_errno_and_is_counted);
  RUN(test_hs_report_leaves_errno_alone_even_when_the_events_descriptor_is_gone);
  RUN(test_existing_signal_handlers_are_left_alone);
  RUN(test_ids_and_keys_are_cut_at_their_limits);
  RUN(test_catalog_line_matches_offline_twin);
  RUN(test_assert_line_length_ignores_counter_values);
  RUN(test_report_has_one_assert_per_id_and_bounded_lines);
  return hs_t_summary();
}
