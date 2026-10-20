#include <errno.h>
#include <time.h>

#include "cl_check.h"

#define MS_PER_S 1000u
#define NS_PER_MS 1000000L
#define POLLS_PER_TICK 20u

void cl_sleep_ms(unsigned ms) {
  struct timespec req = {(time_t)(ms / MS_PER_S), (long)(ms % MS_PER_S) * NS_PER_MS};
  struct timespec rem;
  while (nanosleep(&req, &rem) != 0 && errno == EINTR) req = rem;
}

int cl_trylock_poll(pthread_mutex_t *m, unsigned total_ms) {
  unsigned step = total_ms / POLLS_PER_TICK;
  if (step == 0) step = 1;
  for (unsigned i = 0; i < POLLS_PER_TICK; i++) {
    int rc = pthread_mutex_trylock(m);
    if (rc == 0) return 1;
    if (rc != EBUSY) cl_ok(rc, "pthread_mutex_trylock");
    cl_sleep_ms(step);
  }
  return 0;
}
