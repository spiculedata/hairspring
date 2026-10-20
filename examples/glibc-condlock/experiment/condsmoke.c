/* condsmoke.c: G0 smoke program. Two threads. One pthread_cond_signal / pthread_cond_wait pair on C (the pair under test),
 * plus a handshake pair on C2 so that the signal on C is always sent while the waiter is blocked in pthread_cond_wait.
 * No clock, no sleep, no randomness. Prints one constant line per glibc build. */
#include <gnu/libc-version.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>

#define CHK(x)                                       \
  do {                                               \
    if ((x) != 0) {                                  \
      fputs("condsmoke: " #x " failed\n", stderr);   \
      exit(2);                                       \
    }                                                \
  } while (0)

static pthread_mutex_t M = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t C = PTHREAD_COND_INITIALIZER;
static pthread_cond_t C2 = PTHREAD_COND_INITIALIZER;
static int waiting, ready, woke, signalled;

static void *waiter(void *arg) {
  (void)arg;
  CHK(pthread_mutex_lock(&M));
  waiting = 1;
  CHK(pthread_cond_signal(&C2));
  while (!ready) CHK(pthread_cond_wait(&C, &M));
  woke = 1;
  CHK(pthread_mutex_unlock(&M));
  return NULL;
}

static void *signaller(void *arg) {
  (void)arg;
  CHK(pthread_mutex_lock(&M));
  while (!waiting) CHK(pthread_cond_wait(&C2, &M));
  ready = 1;
  signalled = 1;
  CHK(pthread_cond_signal(&C));
  CHK(pthread_mutex_unlock(&M));
  return NULL;
}

int main(void) {
  pthread_t a, b;
  CHK(pthread_create(&a, NULL, waiter, NULL));
  CHK(pthread_create(&b, NULL, signaller, NULL));
  CHK(pthread_join(a, NULL));
  CHK(pthread_join(b, NULL));
  printf("condsmoke ok libc=%s hdr=%d.%d signalled=%d woke=%d\n", gnu_get_libc_version(), __GLIBC__, __GLIBC_MINOR__, signalled, woke);
  return (signalled && woke) ? 0 : 3;
}
