/* test_audit.c -- the evaluation audit: a call site whose record is not in the catalog section must not be silent.
**
** The record below is a plain static, deliberately outside `hs_catalog_v1`, which is what a compiler that does not
** honour the section attribute produces for some C++ template call sites. Evaluating it moves the thread's
** evaluation count without moving any catalogued counter. The audit compares the two at report time.
**
** The tests share one process and run in order: the loss tests come last because the outstanding evaluations of the
** record outside the catalog stay counted for the rest of the run. */
#include <pthread.h>
#include <stdint.h>
#include <string.h>

#include "../src/hs_internal.h"
#include "hs_test.h"

static struct hs_site rogue = {"audit.rogue", "outside the catalog", __FILE__, __LINE__, HS_KIND_ALWAYS,
                               0, 0, 0, {0, 0, {0, 0, 0, 0}}, {0, 0, {0, 0, 0, 0}}, 0, 0, 0, 0, 0, 0};

static char captured[256][HS_LINE_MAX + 1];
static size_t captured_n;

static void capture(const char *line, size_t len) {
  if (captured_n >= 256 || len > HS_LINE_MAX) return;
  memcpy(captured[captured_n], line, len);
  captured[captured_n++][len] = 0;
}

static void report_into_capture(enum hs_end_reason reason) {
  captured_n = 0;
  hs_report_rearm();
  hs_test_set_sink(capture);
  hs_report_now(reason, 0);
  hs_test_set_sink(NULL);
}

static int saw(const char *needle) {
  for (size_t i = 0; i < captured_n; i++)
    if (strstr(captured[i], needle)) return 1;
  return 0;
}

static const char *end_line(void) {
  for (size_t i = captured_n; i-- > 0;)
    if (strstr(captured[i], "\"kind\":\"end\"")) return captured[i];
  return "";
}

static void catalogued_hit(int ok) { HS_ALWAYS(ok != 0, "audit.catalogued", "a catalogued site", ok); }
static void rogue_hit(int ok) { hs__eval(&rogue, (uint64_t)(ok != 0), HS__VALS(ok)); }

/* ---- the comparison ------------------------------------------------------------------------------------------- */

static void test_nothing_is_missing_when_every_evaluation_was_counted(void) {
  CHECK_U64(hs_audit_missing(10, 10, 0), 0);
  CHECK_U64(hs_audit_missing(0, 0, 0), 0);
}

static void test_surplus_in_the_catalog_is_not_a_loss(void) {
  CHECK_U64(hs_audit_missing(10, 12, 0), 0);
}

static void test_a_deficit_is_missing_beyond_the_in_flight_tolerance(void) {
  CHECK_U64(hs_audit_missing(11, 10, 0), 1);
  CHECK_U64(hs_audit_missing(11, 10, 1), 0); /* one thread may be between its two counter updates */
  CHECK_U64(hs_audit_missing(13, 10, 2), 3);
}

/* ---- the report ----------------------------------------------------------------------------------------------- */

static void *hammer(void *unused) {
  (void)unused;
  for (int i = 0; i < 2000; i++) catalogued_hit(i & 1);
  return NULL;
}

static void test_a_complete_catalog_reports_no_loss_with_several_threads(void) {
  pthread_t t[4];
  for (int i = 0; i < 4; i++) pthread_create(&t[i], NULL, hammer, NULL);
  for (int i = 0; i < 5; i++) catalogued_hit(i & 1);
  for (int i = 0; i < 4; i++) pthread_join(t[i], NULL);
  report_into_capture(HS_END_EXPLICIT);
  CHECK(!saw("incomplete catalog"));
  CHECK(strstr(end_line(), "\"dropped\":                   0") != NULL);
}

static void test_a_site_outside_the_catalog_is_reported_loudly(void) {
  rogue_hit(1);
  rogue_hit(0);
  rogue_hit(1);
  report_into_capture(HS_END_EXPLICIT);
  CHECK(saw("incomplete catalog: 3 evaluation(s)"));
  CHECK(strstr(end_line(), "\"dropped\":                   1") != NULL);
  CHECK(!saw("\"id\":\"audit.rogue\"")); /* it is still absent from the catalog: the loss is announced, not hidden */
}

static void test_the_loss_is_also_reported_after_a_fatal_signal(void) {
  report_into_capture(HS_END_SIGNAL);
  CHECK(saw("incomplete catalog"));
}

int main(void) {
  RUN(test_nothing_is_missing_when_every_evaluation_was_counted);
  RUN(test_surplus_in_the_catalog_is_not_a_loss);
  RUN(test_a_deficit_is_missing_beyond_the_in_flight_tolerance);
  RUN(test_a_complete_catalog_reports_no_loss_with_several_threads);
  RUN(test_a_site_outside_the_catalog_is_reported_loudly);
  RUN(test_the_loss_is_also_reported_after_a_fatal_signal);
  return hs_t_summary();
}
