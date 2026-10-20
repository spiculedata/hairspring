/* Unit tests for the command line parser. */
#include "cl_config.h"
#include "hs_test.h"

static int parse(struct cl_config *c, int argc, char **argv, char *err) { return cl_config_parse(c, argc, argv, err, 128); }

static void test_defaults_without_arguments(void) {
  struct cl_config c;
  char err[128];
  char *argv[] = {"condlock"};
  CHECK(parse(&c, 1, argv, err) == 0);
  CHECK_U64(c.workers, 8);
  CHECK_U64(c.rounds, 500);
  CHECK_U64(c.tick_ms, 100);
  CHECK_U64(c.quorum, 10);
}

static void test_all_four_flags(void) {
  struct cl_config c;
  char err[128];
  char *argv[] = {"condlock", "-n", "4", "-r", "30", "-tick-ms", "20", "-q", "10"};
  CHECK(parse(&c, 9, argv, err) == 0);
  CHECK_U64(c.workers, 4);
  CHECK_U64(c.rounds, 30);
  CHECK_U64(c.tick_ms, 20);
  CHECK_U64(c.quorum, 10);
}

static void test_unknown_flag_is_an_error_with_a_reason(void) {
  struct cl_config c;
  char err[128] = "";
  char *argv[] = {"condlock", "-x", "1"};
  CHECK(parse(&c, 3, argv, err) == -1);
  CHECK(err[0] != '\0');
}

static void test_missing_value_is_an_error(void) {
  struct cl_config c;
  char err[128];
  char *argv[] = {"condlock", "-r"};
  CHECK(parse(&c, 2, argv, err) == -1);
}

static void test_non_numeric_and_signed_values_are_errors(void) {
  struct cl_config c;
  char err[128];
  char *a[] = {"condlock", "-r", "12x"};
  char *b[] = {"condlock", "-r", "-5"};
  char *d[] = {"condlock", "-r", ""};
  char *e[] = {"condlock", "-r", "+5"};
  CHECK(parse(&c, 3, a, err) == -1);
  CHECK(parse(&c, 3, b, err) == -1);
  CHECK(parse(&c, 3, d, err) == -1);
  CHECK(parse(&c, 3, e, err) == -1);
}

static void test_out_of_range_values_are_errors(void) {
  struct cl_config c;
  char err[128];
  char *w0[] = {"condlock", "-n", "0"};
  char *w17[] = {"condlock", "-n", "17"};
  char *r0[] = {"condlock", "-r", "0"};
  char *t0[] = {"condlock", "-tick-ms", "0"};
  char *q0[] = {"condlock", "-q", "0"};
  char *big[] = {"condlock", "-r", "99999999999999999999"};
  CHECK(parse(&c, 3, w0, err) == -1);
  CHECK(parse(&c, 3, w17, err) == -1);
  CHECK(parse(&c, 3, r0, err) == -1);
  CHECK(parse(&c, 3, t0, err) == -1);
  CHECK(parse(&c, 3, q0, err) == -1);
  CHECK(parse(&c, 3, big, err) == -1);
}

static void test_repeated_flag_is_an_error(void) {
  struct cl_config c;
  char err[128];
  char *argv[] = {"condlock", "-r", "5", "-r", "6"};
  CHECK(parse(&c, 5, argv, err) == -1);
}

int main(void) {
  RUN(test_defaults_without_arguments);
  RUN(test_all_four_flags);
  RUN(test_unknown_flag_is_an_error_with_a_reason);
  RUN(test_missing_value_is_an_error);
  RUN(test_non_numeric_and_signed_values_are_errors);
  RUN(test_out_of_range_values_are_errors);
  RUN(test_repeated_flag_is_an_error);
  return hs_t_summary();
}
