/* Stand-in for glibc's public pthread_cond_t, arm U (release/2.40/master at d8a1a1ae, before the lost-wakeup fix), so
 * that src/cl_condraw.c can be compiled and unit tested on a machine without glibc. Transcribed from the real headers
 * in the G0 source tarball (glibc-U.tar, member src/sysdeps/nptl/bits/thread-shared-types.h, sha256
 * 21f6f60e8a28e6e0166f0b3685d939774b553a27d1f221559f2d8176b8fb2358, and src/bits/atomic_wide_counter.h):
 *
 *   typedef union
 *   {
 *     __extension__ unsigned long long int __value64;
 *     struct
 *     {
 *       unsigned int __low;
 *       unsigned int __high;
 *     } __value32;
 *   } __atomic_wide_counter;
 *
 *   struct __pthread_cond_s
 *   {
 *     __atomic_wide_counter __wseq;
 *     __atomic_wide_counter __g1_start;
 *     unsigned int __g_refs[2] __LOCK_ALIGNMENT;
 *     unsigned int __g_size[2];
 *     unsigned int __g1_orig_size;
 *     unsigned int __wrefs;
 *     unsigned int __g_signals[2];
 *   };
 *
 * and, in src/sysdeps/nptl/bits/pthreadtypes.h, pthread_cond_t is a union of `struct __pthread_cond_s __data`, a
 * char array and a long long. __wseq and __g1_start are unions, not integers: only __value64 is an integer.
 * __LOCK_ALIGNMENT is empty here (it is an alignment attribute or empty in the real header). */
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
  unsigned int __g_refs[2] __LOCK_ALIGNMENT;
  unsigned int __g_size[2];
  unsigned int __g1_orig_size;
  unsigned int __wrefs;
  unsigned int __g_signals[2];
};

typedef union
{
  struct __pthread_cond_s __data;
  char __size[48];
  __extension__ long long int __align;
} pthread_cond_t;

#endif
