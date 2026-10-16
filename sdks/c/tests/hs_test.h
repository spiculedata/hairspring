/* hs_test.h -- a tiny test harness: counting checks, one summary, non-zero exit on any failure. */
#ifndef HS_TEST_H
#define HS_TEST_H

#include <stdio.h>
#include <string.h>

static int hs_t_checks;
static int hs_t_failed;

#define CHECK(cond)                                                              \
  do {                                                                           \
    hs_t_checks++;                                                               \
    if (!(cond)) {                                                               \
      hs_t_failed++;                                                             \
      fprintf(stdout, "FAIL %s:%d: %s\n", __FILE__, __LINE__, #cond);            \
    }                                                                            \
  } while (0)

#define CHECK_U64(actual, expected)                                                                          \
  do {                                                                                                       \
    unsigned long long a_ = (unsigned long long)(actual), e_ = (unsigned long long)(expected);               \
    hs_t_checks++;                                                                                           \
    if (a_ != e_) {                                                                                          \
      hs_t_failed++;                                                                                         \
      fprintf(stdout, "FAIL %s:%d: %s == %llu, expected %llu\n", __FILE__, __LINE__, #actual, a_, e_);       \
    }                                                                                                        \
  } while (0)

#define CHECK_STR(actual, expected)                                                                          \
  do {                                                                                                       \
    const char *a_ = (actual), *e_ = (expected);                                                             \
    hs_t_checks++;                                                                                           \
    if (a_ == NULL || strcmp(a_, e_) != 0) {                                                                 \
      hs_t_failed++;                                                                                         \
      fprintf(stdout, "FAIL %s:%d: %s == \"%s\", expected \"%s\"\n", __FILE__, __LINE__, #actual,            \
              a_ ? a_ : "(null)", e_);                                                                       \
    }                                                                                                        \
  } while (0)

#define RUN(test)                    \
  do {                               \
    int before_ = hs_t_failed;       \
    test();                          \
    printf("%s %s\n", hs_t_failed == before_ ? "ok  " : "FAIL", #test); \
  } while (0)

static int hs_t_summary(void) {
  printf("%d checks, %d failed\n", hs_t_checks, hs_t_failed);
  return hs_t_failed == 0 ? 0 : 1;
}

#endif
