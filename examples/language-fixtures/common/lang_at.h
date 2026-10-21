/*
** lang_at.h -- the parts of the lost-update fixtures that C and C++ share. Header only, plain C99 (it also compiles as C++).
**
** The fixture: LANG_AT_THREADS workers each do LANG_AT_ITERS times
**     read the shared counter; spin for LANG_AT_SPIN iterations of user code (no system call); write back read + 1.
** The read and the write are separate relaxed atomic operations, so the program has no data race in the language's
** sense, but an update can be lost. The locked twin holds a mutex across all three steps.
*/
#ifndef LANG_AT_H
#define LANG_AT_H

#include <stdint.h>
#include <stdio.h>
#include <sys/random.h>

#define LANG_AT_THREADS 4u
#define LANG_AT_ITERS 1000u
#define LANG_AT_SPIN 200u /* W: iterations of the busy loop inside the read-modify-write window */
#define LANG_AT_EXPECTED ((uint64_t)LANG_AT_THREADS * LANG_AT_ITERS)

/* The busy loop. The empty asm with a memory clobber keeps the optimiser from removing the loop, unrolling it away,
** or moving the counter's load and store across it, whatever the compiler and the optimisation level. */
static inline uint64_t lang_at_spin(uint64_t seed, unsigned rounds) {
  uint64_t acc = seed;
  for (unsigned i = 0; i < rounds; i++) {
    acc = acc * 6364136223846793005ull + 1442695040888963407ull;
    __asm__ volatile("" : "+r"(acc) : : "memory");
  }
  return acc;
}

/* FNV-1a over the order in which the workers finished (one byte per worker). */
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

/* The one stdout line: fixed width, so its length does not depend on the outcome. */
static inline void lang_at_print(const char *lang, uint64_t final_value, uint64_t order, uint64_t random_value) {
  printf("lang=%-3s final=%010llu order=%016llx rand=%016llx\n", lang, (unsigned long long)final_value,
         (unsigned long long)order, (unsigned long long)random_value);
}

#endif
