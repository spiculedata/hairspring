/* cl_check.h -- time shim and the pthread_ok harness site.
 *
 * No worker reads a clock. The only sleeper is the watchdog (cl_sleep_ms). macOS has no pthread_mutex_timedlock, so the
 * watchdog takes the lock mutex by trylock plus a short sleep, on every platform (cl_trylock_poll): it must never
 * block on a mutex that a thread blocked forever may hold. */
#ifndef CL_CHECK_H
#define CL_CHECK_H

#include <pthread.h>

#define CL_EXIT_HARNESS 5

/* Evaluates the site condlock.harness.pthread_ok for one pthread call; on failure reports and exits 5. */
void cl_ok(int rc, const char *what);

/* nanosleep for ms milliseconds (restarted on EINTR). */
void cl_sleep_ms(unsigned ms);

/* Try to take *m for up to about total_ms milliseconds without ever blocking untimed. 1 = locked, 0 = not acquired. */
int cl_trylock_poll(pthread_mutex_t *m, unsigned total_ms);

#endif
