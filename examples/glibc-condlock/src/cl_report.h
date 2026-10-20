/* cl_report.h -- the one-line, fixed-width stdout report and the state dump written to stderr on a hit or stall. */
#ifndef CL_REPORT_H
#define CL_REPORT_H

#include <stddef.h>
#include <stdint.h>

#include "cl_rule.h"

#define CL_LINE_MAX 384

/* Counters printed on stdout. Counters are right-aligned in 8 characters; no time, no address. */
struct cl_report {
  const char *libc; /* gnu_get_libc_version() on glibc, "none" elsewhere */
  const char *hdr;  /* "<__GLIBC__>.<__GLIBC_MINOR__>" on glibc, "none" elsewhere */
  uint64_t rounds_done;
  uint64_t signals;        /* releases (a skipped signal in a planted build still counts) */
  uint64_t signals_blocked; /* releases with |B| >= 1 */
  uint64_t max_blocked;
  uint64_t ticks;
  uint64_t spanned;        /* waits that spanned two releases */
  uint32_t max_stagnant;
  int has_applied;         /* planted builds only */
  int applied;
};

/* Writes the line (no newline) into buf; returns its length, or -1 if it does not fit. */
int cl_report_format(char *buf, size_t len, const struct cl_report *r);

/* Raw condvar internals for the dump. Which fields are meaningful depends on arm. */
enum cl_arm { CL_ARM_NONE = 0, CL_ARM_U = 1, CL_ARM_C = 2 };

struct cl_condraw {
  enum cl_arm arm;
  uint64_t wseq, g1_start;
  uint32_t g_refs[2];  /* arm U only */
  uint32_t g_size[2];
  uint32_t g1_orig_size, wrefs;
  uint32_t g_signals[2];
};

struct cl_dump {
  const char *what; /* "HIT" or "STALL" */
  enum cl_subtype subtype;
  uint64_t s_seq, progress, ticks;
  uint32_t blocked_n, stagnant, waiting_n;
  int is_locked;
  struct cl_condraw raw;
};

/* One plain-text line, starting with "condlock: ". Returns its length, or -1 if it does not fit. */
int cl_dump_format(char *buf, size_t len, const struct cl_dump *d);

/* Reads the lock condvar's raw fields when the build is for glibc arm U or C (see cl_condraw.c); else arm NONE. */
struct cl_condraw cl_condraw_read(const void *cond);

#endif
