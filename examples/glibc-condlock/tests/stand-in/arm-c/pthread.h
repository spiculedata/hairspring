/* Stand-in for glibc's public pthread_cond_t, arm C (release/2.40/master at 8d3dd23e, with the lost-wakeup fix series),
 * so that src/cl_condraw.c can be compiled and unit tested on a machine without glibc. Transcribed from the real
 * headers in the G0 source tarball (glibc-C.tar, member src/sysdeps/nptl/bits/thread-shared-types.h, sha256
 * f190b40265292ac933acdbae022b2b5afb943dd2ea5e70c40adbf85296da2fdc; __atomic_wide_counter is the same as in arm U):
 *
 *   struct __pthread_cond_s
 *   {
 *     __atomic_wide_counter __wseq;
 *     __atomic_wide_counter __g1_start;
 *     unsigned int __g_size[2] __LOCK_ALIGNMENT;
 *     unsigned int __g1_orig_size;
 *     unsigned int __wrefs;
 *     unsigned int __g_signals[2];
 *     unsigned int __unused_initialized_1;
 *     unsigned int __unused_initialized_2;
 *   };
 *
 * There is no __g_refs in this layout. __LOCK_ALIGNMENT is empty here. */
#ifndef STAND_IN_PTHREAD_H
#define STAND_IN_PTHREAD_H

#define __LOCK_ALIGNMENT

typedef union
{
  __extension__ unsigned long long int __value64;
  struct
  {
    unsigned int __low;
    unsigned int __high;
  } __value32;
} __atomic_wide_counter;

struct __pthread_cond_s
{
  __atomic_wide_counter __wseq;
  __atomic_wide_counter __g1_start;
  unsigned int __g_size[2] __LOCK_ALIGNMENT;
  unsigned int __g1_orig_size;
  unsigned int __wrefs;
  unsigned int __g_signals[2];
  unsigned int __unused_initialized_1;
  unsigned int __unused_initialized_2;
};

typedef union
{
  struct __pthread_cond_s __data;
  char __size[48];
  __extension__ long long int __align;
} pthread_cond_t;

#endif
