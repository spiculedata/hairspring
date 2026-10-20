/* cl_lock.h -- the lock (mutex G + condvar L + is_locked) and the barrier (mutex B + condvar K). */
#ifndef CL_LOCK_H
#define CL_LOCK_H

#include "cl_ctx.h"

/* Blocks (untimed pthread_cond_wait on L) while the lock is held; then takes it. */
void cl_acquire(struct cl_ctx *c, unsigned id);

/* Frees the lock, records the snapshot, and signals L unconditionally (unless a planted build says otherwise). */
void cl_release(struct cl_ctx *c, unsigned id);

/* Round barrier: the last arriver broadcasts K, the others wait on it. */
void cl_barrier(struct cl_ctx *c, unsigned id);

#endif
