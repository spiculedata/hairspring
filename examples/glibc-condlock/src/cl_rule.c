#include "cl_rule.h"

struct cl_snapshot cl_snapshot_take(uint64_t seq, const uint8_t *waiting, const uint64_t *returns, unsigned n) {
  struct cl_snapshot s = {0};
  s.seq = seq;
  for (unsigned i = 0; i < n && i < CL_MAX_WORKERS; i++) {
    s.blocked[i] = waiting[i] ? 1u : 0u;
    s.blocked_n += s.blocked[i];
    s.ret_at[i] = returns[i];
  }
  return s;
}

int cl_attributed_stuck(int is_locked, const struct cl_snapshot *snap, const uint8_t *waiting, const uint64_t *returns,
                        unsigned n) {
  if (is_locked || snap->blocked_n == 0) return 0;
  for (unsigned i = 0; i < n && i < CL_MAX_WORKERS; i++) {
    if (!snap->blocked[i]) continue;
    if (!waiting[i] || returns[i] != snap->ret_at[i]) return 0;
  }
  return 1;
}

enum cl_subtype cl_classify_stall(int have_lock, int is_locked, const uint8_t *waiting, unsigned n) {
  if (!have_lock) return CL_SUB_M;
  if (is_locked) return CL_SUB_L;
  for (unsigned i = 0; i < n && i < CL_MAX_WORKERS; i++) {
    if (waiting[i]) return CL_SUB_Q;
  }
  return CL_SUB_B;
}

uint32_t cl_stag_update(struct cl_stag *s, uint64_t progress) {
  s->stagnant = (progress == s->last) ? s->stagnant + 1u : 0u;
  s->last = progress;
  if (s->stagnant > s->max_stagnant) s->max_stagnant = s->stagnant;
  return s->stagnant;
}
