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

#elif defined(HS_SCENARIO_small)

/* Same shape as "big" but every line stays under 512 bytes (macOS PIPE_BUF). */
#define ROW(X, p) X(p##0) X(p##1) X(p##2) X(p##3) X(p##4) X(p##5) X(p##6) X(p##7)
#define SITES(X) ROW(X, a) ROW(X, b) ROW(X, c) ROW(X, d) ROW(X, e) ROW(X, f) ROW(X, g) ROW(X, h)
#define SMALL_SITE(n) HS_REACHABLE("small.site." #n, "short");

int main(void) {
  for (int rnd = 0; rnd < 3; rnd++) {
    SITES(SMALL_SITE)
  }
  return 0;
}

#else
#error "define one HS_SCENARIO_<name>"
#endif
