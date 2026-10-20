/* cl_sdk.h -- the Hairspring C SDK, or (with -DCONDLOCK_NO_SDK) a violation-only stand-in.
 *
 * The stand-in prints `HAIRSPRING_VIOLATION: <id>` when an `always` is false or an `unreachable` is reached, and does
 * nothing else: no catalog, no report, no events. It exists to compare detection with and without the SDK's writes. */
#ifndef CL_SDK_H
#define CL_SDK_H

#ifdef CONDLOCK_NO_SDK

#include <stdio.h>
#include <stdlib.h>

static inline void cl_violation(const char *id) { fprintf(stderr, "HAIRSPRING_VIOLATION: %s\n", id); }

#define HS_ALWAYS(cond, id, msg, ...) \
  do {                                \
    if (!(cond)) cl_violation(id);    \
  } while (0)
#define HS_UNREACHABLE(id, msg, ...) cl_violation(id)
#define HS_SOMETIMES(cond, id, msg, ...) ((void)(cond))
#define HS_REACHABLE(id, msg, ...) ((void)0)
#define HS_WORK(n) ((void)(n))
static inline int hs_thread(const char *label, unsigned index) {
  (void)label;
  (void)index;
  return 0;
}
static inline void hs_exit(int status) __attribute__((noreturn));
static inline void hs_exit(int status) { exit(status); }

#else

#include "hairspring.h"

#endif

#endif
