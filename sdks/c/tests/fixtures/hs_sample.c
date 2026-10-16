/* hs_sample.c -- a sample instrumented program, built once per scenario: -DHS_SCENARIO_<name>.
**
** A binary's catalog holds every assertion compiled into it, so each scenario is its own binary and its report
** contains only that scenario's ids. tests/test_proc.c and the Rust conformance test run them and inspect the
** events they write. Usage: hs_sample_<name> [n]
*/
#include <pthread.h>
#include <signal.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../../include/hairspring.h"

#if defined(HS_SCENARIO_basic)

int main(int argc, char **argv) {
  (void)argv;
  for (int i = 0; i < 5; i++) HS_ALWAYS(1, "basic.always.ok", "always holds", i, 2);
  HS_ALWAYS(0, "basic.always.bad", "always violated", 7, 8);
  for (int i = 0; i < 4; i++) HS_SOMETIMES(i % 2 == 0, "basic.sometimes", "sometimes holds", i);
  HS_REACHABLE("basic.reach.hit", "is reached");
  if (argc > 99) { /* never taken: these sites exist only in the catalog */
    HS_REACHABLE("basic.reach.never", "never reached");
    HS_UNREACHABLE("basic.unreach.clean", "never reached either");
    HS_ALWAYS_OR_UNREACHABLE(1, "basic.aou.never", "never evaluated");
  }
  HS_WORK(300);
  return 0;
}

#elif defined(HS_SCENARIO_merge)

static void __attribute__((noinline)) merge_a(int ok) { HS_SOMETIMES(ok, "merge.shared", "one id, two sites", 1); }
static void __attribute__((noinline)) merge_b(int ok) { HS_SOMETIMES(ok, "merge.shared", "one id, two sites", 2); }

int main(void) {
  merge_a(1);
  merge_a(0);
  merge_b(1);
  merge_b(1);
  HS_REACHABLE("merge.other", "a second, distinct id");
  return 0;
}

#elif defined(HS_SCENARIO_fork)

int main(void) {
  for (int i = 0; i < 2; i++) HS_REACHABLE("fork.parent", "parent reaches this");
  for (int child = 0; child < 2; child++) {
    pid_t pid = fork();
    if (pid == 0) {
      for (int i = 0; i < 3; i++) HS_REACHABLE("fork.child", "child reaches this");
      exit(0);
    }
    int status = 0;
    waitpid(pid, &status, 0);
  }
  return 0;
}

#elif defined(HS_SCENARIO_threads)

static void *thread_body(void *arg) {
  unsigned idx = (unsigned)(uintptr_t)arg;
  hs_thread("w", idx);
  HS_SOMETIMES(1, "threads.each", "every thread evaluates", idx);
  return NULL;
}

int main(int argc, char **argv) {
  int n = argc > 1 ? atoi(argv[1]) : 0;
  HS_WORK(1);
  for (int i = 0; i < n; i++) {
    pthread_t t;
    pthread_create(&t, NULL, thread_body, (void *)(uintptr_t)i);
    pthread_join(t, NULL);
  }
  return 0;
}

#elif defined(HS_SCENARIO_crash)

int main(void) {
  HS_ALWAYS(1, "crash.before", "evaluated before the crash");
  HS_REACHABLE("crash.reach", "reached before the crash");
  raise(SIGSEGV);
  return 0;
}

#elif defined(HS_SCENARIO_exit_status)

int main(void) {
  HS_REACHABLE("exit.reach", "reached");
  hs_exit(3);
}

#elif defined(HS_SCENARIO_eval)

int main(void) {
  HS_EVAL_ALWAYS(1, "eval.always");
  HS_EVAL_SOMETIMES(0, "eval.sometimes");
  HS_EVAL_REACHABLE("eval.reach");
  return 0;
}

#elif defined(HS_SCENARIO_big)

#define LONG_MSG                                                                                   \
  "a deliberately long message so that each catalog line is large and two processes writing at "   \
  "once have a real chance of interleaving if a line were ever split across write calls; padding " \
  "padding padding padding padding padding padding padding padding padding padding padding "       \
  "padding padding padding padding padding padding padding padding padding padding padding"

#define ROW(X, p) X(p##0) X(p##1) X(p##2) X(p##3) X(p##4) X(p##5) X(p##6) X(p##7)
#define SITES(X) ROW(X, a) ROW(X, b) ROW(X, c) ROW(X, d) ROW(X, e) ROW(X, f) ROW(X, g) ROW(X, h)
#define BIG_SITE(n) HS_SOMETIMES(rnd % 2 == 0, "big.site." #n, LONG_MSG, rnd);

int main(void) {
  for (int rnd = 0; rnd < 3; rnd++) {
    SITES(BIG_SITE)
  }
  return 0;
}

#elif defined(HS_SCENARIO_forkexec)

/* Usage: hs_sample_forkexec <image-to-exec>. The child execs the image: an instrumented one starts life under the
** child's tag; an uninstrumented one must leave nothing behind. */
int main(int argc, char **argv) {
  if (argc < 2) return 2;
  HS_REACHABLE("forkexec.parent", "parent reaches this");
  pid_t pid = fork();
  if (pid == 0) {
    execl(argv[1], argv[1], (char *)NULL);
    _exit(127);
  }
  int status = 0;
  waitpid(pid, &status, 0);
  return 0;
}

#elif defined(HS_SCENARIO_spawn)

#include <spawn.h>
extern char **environ;

/* Usage: hs_sample_spawn <image>. posix_spawn runs no fork handlers, so the child must tell itself apart. */
int main(int argc, char **argv) {
  if (argc < 2) return 2;
  HS_REACHABLE("spawn.parent", "parent reaches this");
  pid_t pid = 0;
  char *args[] = {argv[1], NULL};
  if (posix_spawn(&pid, argv[1], NULL, NULL, args, environ) != 0) return 3;
  int status = 0;
  waitpid(pid, &status, 0);
  return 0;
}

#elif defined(HS_SCENARIO_closefd)

#include <fcntl.h>

/* Usage: hs_sample_closefd [data-file]. Closes stderr; with a file, reuses descriptor 2 for it. */
int main(int argc, char **argv) {
  HS_REACHABLE("closefd.reach", "reached");
  close(2);
  if (argc > 1) {
    int fd = open(argv[1], O_WRONLY | O_CREAT | O_TRUNC, 0644);
    if (fd >= 0 && write(fd, "application data\n", 17) != 17) return 3;
  }
  return 0;
}

#elif defined(HS_SCENARIO_manyids)

#define L1(M, P) M(P##0) M(P##1) M(P##2) M(P##3) M(P##4) M(P##5) M(P##6) M(P##7)
#define L2(M, P) L1(M, P##0) L1(M, P##1) L1(M, P##2) L1(M, P##3) L1(M, P##4) L1(M, P##5) L1(M, P##6) L1(M, P##7)
#define L3(M, P) L2(M, P##0) L2(M, P##1) L2(M, P##2) L2(M, P##3) L2(M, P##4) L2(M, P##5) L2(M, P##6) L2(M, P##7)
#define L4(M, P) L3(M, P##0) L3(M, P##1) L3(M, P##2) L3(M, P##3) L3(M, P##4) L3(M, P##5) L3(M, P##6) L3(M, P##7)
#define MANY_SITE(n) HS_REACHABLE("many." #n, "m");

/* One function per 64 sites: gcc compiles thousands of static locals in a single function very slowly. */
#define I1(M, P) M(P##0) M(P##1) M(P##2) M(P##3) M(P##4) M(P##5) M(P##6) M(P##7)
#define I2(M, P) I1(M, P##0) I1(M, P##1) I1(M, P##2) I1(M, P##3) I1(M, P##4) I1(M, P##5) I1(M, P##6) I1(M, P##7)
#define CHUNK(P) static void chunk_##P(void) { I2(MANY_SITE, P) }
#define CALL(P) chunk_##P();

L2(CHUNK, a)
L1(CHUNK, b)

/* 4608 distinct ids: over the per-process limit of 4096. */
int main(void) {
  L2(CALL, a)
  L1(CALL, b)
  return 0;
}

#elif defined(HS_SCENARIO_conflict)

int main(void) {
  HS_ALWAYS(1, "conflict.id", "declared as always");
  HS_SOMETIMES(1, "conflict.id", "declared as sometimes");
  return 0;
}

#elif defined(HS_SCENARIO_empty)

int main(void) {
  HS_WORK(1); /* links the SDK in, with no assertion at all */
  return 0;
}

#elif defined(HS_SCENARIO_limits)

#define X10 "xxxxxxxxxx"
#define X100 X10 X10 X10 X10 X10 X10 X10 X10 X10 X10
#define M512 X100 X100 X100 X100 X100 X10 "xx"

/* An id and a message exactly at their limits, and an id with characters the protocol does not allow. */
int main(void) {
  HS_REACHABLE(X100 X10 X10 "xxxxxxxx", M512);
  HS_REACHABLE("bad id$with:odd/chars", "quote \" and backslash \\ in the message");
  return 0;
}

#elif defined(HS_SCENARIO_plain_exit)

int main(void) {
  HS_REACHABLE("plain.reach", "reached");
  return 7;
}

#elif defined(HS_SCENARIO_segv)

int main(void) {
  HS_REACHABLE("segv.before", "reached before the fault");
  *(volatile int *)0 = 1;
  return 0;
}

#elif defined(HS_SCENARIO_abort)

int main(void) {
  HS_REACHABLE("abort.before", "reached before abort");
  abort();
}

#elif defined(HS_SCENARIO_threadsegv)

static void *faulting(void *arg) {
  (void)arg;
  hs_thread("faulty", 0);
  HS_REACHABLE("threadsegv.worker", "reached in the worker");
  *(volatile int *)0 = 1;
  return NULL;
}

int main(void) {
  HS_REACHABLE("threadsegv.main", "reached in main");
  pthread_t t;
  pthread_create(&t, NULL, faulting, NULL);
  pthread_join(t, NULL);
  return 0;
}

#elif defined(HS_SCENARIO_alive)

#include <time.h>

static volatile int g_stop;

static void *spinner(void *arg) {
  (void)arg;
  while (!g_stop) HS_SOMETIMES(1, "alive.spin", "spinning");
  return NULL;
}

static void *short_lived(void *arg) {
  (void)arg;
  HS_WORK(1);
  return NULL;
}

static void *spawner(void *arg) {
  (void)arg;
  while (!g_stop) {
    pthread_t t;
    if (pthread_create(&t, NULL, short_lived, NULL) == 0) pthread_detach(t);
    struct timespec ts = {0, 20000};
    nanosleep(&ts, NULL);
  }
  return NULL;
}

/* Threads are still evaluating, and new ones still starting, when the process exits. */
int main(void) {
  pthread_t t;
  for (int i = 0; i < 4; i++) pthread_create(&t, NULL, spinner, NULL);
  pthread_create(&t, NULL, spawner, NULL);
  struct timespec ts = {0, 3000000};
  nanosleep(&ts, NULL);
  return 0;
}

#elif defined(HS_SCENARIO_alarm)

#include <sys/time.h>

static void tick(int sig) { (void)sig; }

#define ROW(X, p) X(p##0) X(p##1) X(p##2) X(p##3) X(p##4) X(p##5) X(p##6) X(p##7)
#define SITES(X) ROW(X, a) ROW(X, b) ROW(X, c) ROW(X, d) ROW(X, e) ROW(X, f) ROW(X, g) ROW(X, h)
#define ALARM_SITE(n) HS_SOMETIMES(1, "alarm.site." #n, "short");

/* A timer signal (no SA_RESTART) keeps arriving while the report is written. */
int main(void) {
  struct sigaction sa;
  memset(&sa, 0, sizeof sa);
  sa.sa_handler = tick;
  sigaction(SIGALRM, &sa, NULL);
  struct itimerval it = {{0, 50}, {0, 50}};
  setitimer(ITIMER_REAL, &it, NULL);
  SITES(ALARM_SITE)
  return 0;
}

#else
#error "define one HS_SCENARIO_<name>"
#endif
