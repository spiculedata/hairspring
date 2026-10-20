/* cl_condraw.c -- raw fields of the lock condvar for the state dump.
 *
 * Only a glibc build for arm U (-DCONDLOCK_ARM_U, the pre-fix layout with __g_refs) or arm C (-DCONDLOCK_ARM_C, the
 * fixed layout without it) reads the public `__data` view of pthread_cond_t. Everywhere else the arm is NONE.
 * The glibc branches cannot be compiled on macOS: they are compile-checked only when built on Linux against glibc. */
#include "cl_report.h"

#if defined(CONDLOCK_ARM_U) && defined(CONDLOCK_ARM_C)
#error "define at most one of CONDLOCK_ARM_U and CONDLOCK_ARM_C"
#endif

#if defined(__GLIBC__) && (defined(CONDLOCK_ARM_U) || defined(CONDLOCK_ARM_C))

#include <pthread.h>

struct cl_condraw cl_condraw_read(const void *cond) {
  const pthread_cond_t *c = (const pthread_cond_t *)cond;
  struct cl_condraw raw = {0};
  raw.wseq = c->__data.__wseq.__value64; /* __atomic_wide_counter is a union; __value64 is its 64-bit view */
  raw.g1_start = c->__data.__g1_start.__value64;
  raw.g_size[0] = c->__data.__g_size[0];
  raw.g_size[1] = c->__data.__g_size[1];
  raw.g1_orig_size = c->__data.__g1_orig_size;
  raw.wrefs = c->__data.__wrefs;
  raw.g_signals[0] = c->__data.__g_signals[0];
  raw.g_signals[1] = c->__data.__g_signals[1];
#if defined(CONDLOCK_ARM_U)
  raw.arm = CL_ARM_U;
  raw.g_refs[0] = c->__data.__g_refs[0];
  raw.g_refs[1] = c->__data.__g_refs[1];
#else
  raw.arm = CL_ARM_C;
#endif
  return raw;
}

#else

struct cl_condraw cl_condraw_read(const void *cond) {
  struct cl_condraw raw = {0};
  (void)cond;
  raw.arm = CL_ARM_NONE;
  return raw;
}

#endif
