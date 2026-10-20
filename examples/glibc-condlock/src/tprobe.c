/* tprobe: the engine's time-model probe (run natively here only to test the program itself).
 *
 *   tprobe i   <secs>   thread A nanosleeps <secs>; thread B is blocked in pthread_cond_wait (a futex wait). After its sleep
 *                       A wakes B, so both exit 0 (B is never the last, indefinitely blocked thread).
 *   tprobe ii  <K>      K ping-pongs between two threads through a mutex and a condvar (turn taking).
 *   tprobe iii <secs>   as i, but A sleeps with the raw clock_nanosleep(CLOCK_MONOTONIC, 0, ...) system call. Linux only:
 *                       elsewhere it prints NOT-RUN and exits 77.
 *
 * No SDK, no clock reads. The measurement (wall time and virtual time under the engine) is taken from outside. */
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#ifdef __linux__
#include <sys/syscall.h>
#include <unistd.h>
#endif

#define EXIT_USAGE 64
#define EXIT_NOT_RUN 77
#define EXIT_FAILURE_CODE 2
#define MAX_SECS 3600u
#define MAX_PINGPONGS 1000000u

struct probe {
  pthread_mutex_t m;
  pthread_cond_t cv;
  int woken;     /* case i / iii: A has woken B */
  unsigned turn; /* case ii: whose turn it is, 0 or 1 */
  unsigned rounds;
  unsigned secs;
  int raw; /* sleep with the raw system call */
};

static void chk(int rc, const char *what) {
  if (rc != 0) {
    fprintf(stderr, "tprobe: %s failed (%d)\n", what, rc);
    exit(EXIT_FAILURE_CODE);
  }
}

static void sleep_secs(const struct probe *p) {
  struct timespec ts = {(time_t)p->secs, 0};
#ifdef __linux__
  if (p->raw) {
    if (syscall(SYS_clock_nanosleep, CLOCK_MONOTONIC, 0, &ts, NULL) != 0) {
      fputs("tprobe: clock_nanosleep failed\n", stderr);
      exit(EXIT_FAILURE_CODE);
    }
    return;
  }
#endif
  while (nanosleep(&ts, &ts) != 0) {
  }
}

static void *sleeper(void *arg) {
  struct probe *p = (struct probe *)arg;
  sleep_secs(p);
  chk(pthread_mutex_lock(&p->m), "lock");
  p->woken = 1;
  chk(pthread_cond_signal(&p->cv), "signal");
  chk(pthread_mutex_unlock(&p->m), "unlock");
  return NULL;
}

static void *blocked(void *arg) {
  struct probe *p = (struct probe *)arg;
  chk(pthread_mutex_lock(&p->m), "lock");
  while (!p->woken) chk(pthread_cond_wait(&p->cv, &p->m), "wait");
  chk(pthread_mutex_unlock(&p->m), "unlock");
  return NULL;
}

struct player_arg {
  struct probe *p;
  unsigned me;
};

static void *ping_pong(void *arg) {
  struct player_arg *a = (struct player_arg *)arg;
  struct probe *p = a->p;
  for (unsigned i = 0; i < p->rounds; i++) {
    chk(pthread_mutex_lock(&p->m), "lock");
    while (p->turn != a->me) chk(pthread_cond_wait(&p->cv, &p->m), "wait");
    p->turn = 1u - a->me;
    chk(pthread_cond_signal(&p->cv), "signal");
    chk(pthread_mutex_unlock(&p->m), "unlock");
  }
  return NULL;
}

static int parse(const char *s, unsigned max, unsigned *out) {
  if (s == NULL || *s == '\0') return -1;
  unsigned long v = 0;
  for (; *s != '\0'; s++) {
    if (*s < '0' || *s > '9') return -1;
    v = v * 10ul + (unsigned long)(*s - '0');
    if (v > max) return -1;
  }
  if (v == 0) return -1;
  *out = (unsigned)v;
  return 0;
}

static int usage(void) {
  fputs("tprobe: usage: tprobe i <secs> | ii <K> | iii <secs>\n", stderr);
  return EXIT_USAGE;
}

static int run_sleep_case(struct probe *p, const char *name) {
  pthread_t a, b;
  chk(pthread_create(&b, NULL, blocked, p), "create B");
  chk(pthread_create(&a, NULL, sleeper, p), "create A");
  chk(pthread_join(a, NULL), "join A");
  chk(pthread_join(b, NULL), "join B");
  printf("tprobe %s ok slept=%u woke=%d\n", name, p->secs, p->woken);
  return p->woken ? 0 : EXIT_FAILURE_CODE;
}

static int run_ping_pong(struct probe *p) {
  pthread_t t[2];
  struct player_arg args[2] = {{p, 0}, {p, 1}};
  for (unsigned i = 0; i < 2; i++) chk(pthread_create(&t[i], NULL, ping_pong, &args[i]), "create player");
  for (unsigned i = 0; i < 2; i++) chk(pthread_join(t[i], NULL), "join player");
  printf("tprobe ii ok pingpongs=%u\n", p->rounds);
  return 0;
}

int main(int argc, char **argv) {
  if (argc != 3) return usage();
  struct probe p;
  memset(&p, 0, sizeof p);
  chk(pthread_mutex_init(&p.m, NULL), "init mutex");
  chk(pthread_cond_init(&p.cv, NULL), "init cond");
  if (strcmp(argv[1], "ii") == 0) {
    if (parse(argv[2], MAX_PINGPONGS, &p.rounds) != 0) return usage();
    return run_ping_pong(&p);
  }
  int raw = strcmp(argv[1], "iii") == 0;
  if (!raw && strcmp(argv[1], "i") != 0) return usage();
  if (parse(argv[2], MAX_SECS, &p.secs) != 0) return usage();
#ifndef __linux__
  if (raw) {
    puts("tprobe iii NOT-RUN: the raw clock_nanosleep system call exists only on Linux");
    return EXIT_NOT_RUN;
  }
#endif
  p.raw = raw;
  return run_sleep_case(&p, argv[1]);
}
