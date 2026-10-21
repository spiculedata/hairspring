/*
** lang_at.h -- the parts of the lost-update fixture that C and C++ share. Header only, plain C99 (it also compiles as C++).
**
** One program per language takes its mode as argv[1]: buggy, fixed or wide. LANG_AT_THREADS workers each do
** LANG_AT_ITERS times
**     read the shared counter; spin for the mode's number of rounds of user code (no system call); write back read + 1.
** The read and the write are separate relaxed atomic operations, so the program has no data race in the language's
** sense, but an update can be lost. Fixed holds a mutex across all three steps; wide is buggy with a longer busy loop.
** The mode is chosen at run time: one artifact, one sha256, three arms.
*/
#ifndef LANG_AT_H
#define LANG_AT_H

#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/random.h>
#include <unistd.h>

#define LANG_AT_THREADS 4u
#define LANG_AT_ITERS 1000u
#define LANG_AT_SPIN 1000u       /* W: iterations of the busy loop inside the read-modify-write window */
#define LANG_AT_SPIN_WIDE 10000u /* W x 10, the wide mode */
#define LANG_AT_EXPECTED ((uint64_t)LANG_AT_THREADS * LANG_AT_ITERS)

#define LANG_AT_EXIT_INVARIANT 3 /* a thread did not start or finish, or the result line could not be written */
#define LANG_AT_EXIT_USAGE 64

struct lang_at_mode {
  const char *name;
  int locked;      /* a mutex is held across the read-modify-write */
  unsigned spin;   /* rounds of the busy loop inside the window */
};

/* The three modes. wide differs from buggy only in the length of the busy loop. */
static const struct lang_at_mode lang_at_modes[] = {
    {"buggy", 0, LANG_AT_SPIN},
    {"fixed", 1, LANG_AT_SPIN},
    {"wide", 0, LANG_AT_SPIN_WIDE},
};

/* The mode named by argv[1], or NULL after a usage line on stderr (no argument, an unknown mode, a spare argument). */
static inline const struct lang_at_mode *lang_at_parse_mode(int argc, char **argv) {
  if (argc == 2) {
    for (unsigned i = 0; i < sizeof lang_at_modes / sizeof lang_at_modes[0]; i++) {
      if (strcmp(argv[1], lang_at_modes[i].name) == 0) return &lang_at_modes[i];
    }
  }
  fputs("usage: lost_update buggy|fixed|wide\n", stderr);
  return NULL;
}

/* The busy loop. The empty asm with a memory clobber keeps the optimiser from removing the loop, unrolling it away,
** or moving the counter's load and store across it, whatever the compiler and the optimisation level. It is kept
** out of line so that the window is one call between the load and the store, and so that it has a symbol to inspect. */
static __attribute__((noinline)) uint64_t lang_at_spin(uint64_t seed, unsigned rounds) {
  uint64_t acc = seed;
  for (unsigned i = 0; i < rounds; i++) {
    acc = acc * 6364136223846793005ull + 1442695040888963407ull;
    __asm__ volatile("" : "+r"(acc) : : "memory");
  }
  return acc;
}

/* One sink per worker, on its own cache line, written after the window: the busy loop's result stays live. */
struct lang_at_sink {
  uint64_t value;
  char pad[56];
};
static struct lang_at_sink lang_at_sinks[LANG_AT_THREADS];

static inline void lang_at_keep(unsigned worker, uint64_t value) {
  __atomic_store_n(&lang_at_sinks[worker].value, value, __ATOMIC_RELAXED);
}

/* FNV-1a over the arrival list: one ASCII digit per worker, in the order the workers finished. */
static inline uint64_t lang_at_order_hash(const uint8_t *order, unsigned n) {
  uint64_t h = 14695981039346656037ull;
  for (unsigned i = 0; i < n; i++) {
    h ^= order[i];
    h *= 1099511628211ull;
  }
  return h;
}

/* One value from the system's entropy source, so a run that leaks entropy into its output shows it. */
static inline uint64_t lang_at_random(void) {
  uint64_t r = 0;
  if (getentropy(&r, sizeof r) != 0) r = 0;
  return r;
}

/* The one stdout line, fixed width so its length does not depend on the outcome, written with one write call.
** Returns 0, or -1 when the line did not fit or was not written whole. */
static inline int lang_at_print(const char *lang, const char *mode, uint64_t final_value, uint64_t order,
                                uint64_t random_value) {
  char line[160];
  int n = snprintf(line, sizeof line, "LANGAT v1 lang=%s mode=%s final=%020llu expect=%020llu order=%016llx rand=%016llx\n",
                   lang, mode, (unsigned long long)final_value, (unsigned long long)LANG_AT_EXPECTED,
                   (unsigned long long)order, (unsigned long long)random_value);
  if (n <= 0 || (size_t)n >= sizeof line) return -1;
  return write(STDOUT_FILENO, line, (size_t)n) == n ? 0 : -1;
}

#endif
