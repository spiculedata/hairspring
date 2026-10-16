/* test_nosyscall.c -- plan test T1 (Linux only): call sites make no system call.
**
** A fresh thread installs a seccomp-BPF filter that allows only exit, exit_group and rt_sigreturn, then runs a
** million evaluations of every kind (true and false, first evaluations and first-thread slot claim included), HS_WORK
** and hs_thread, and ends with a raw exit. Any other system call (a stray getenv-triggered read, a malloc-driven mmap,
** a futex) kills the process, and the parent sees a signal instead of a clean exit.
*/
#if !defined(__linux__)
#include <stdio.h>
int main(void) {
  printf("skip: seccomp is Linux-only\n");
  return 0;
}
#else
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <pthread.h>
#include <stddef.h>
#include <stdio.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#include "../include/hairspring.h"

#define EVALUATIONS 1000000

static void allow_only_exit(void) {
  struct sock_filter prog[] = {
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_exit, 3, 0),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_exit_group, 2, 0),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_rt_sigreturn, 1, 0),
      BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
      BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
  };
  struct sock_fprog fprog = {.len = sizeof prog / sizeof prog[0], .filter = prog};
  if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 || prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &fprog) != 0) {
    _exit(90); /* the filter could not be installed: do not claim a pass */
  }
}

static void *asserting_thread(void *arg) {
  (void)arg;
  volatile unsigned flip = 0;
  allow_only_exit();
  for (unsigned i = 0; i < EVALUATIONS; i++) {
    flip = flip + 1u;
    int ok = (int)(flip & 1u);
    HS_ALWAYS(ok, "nosys.always", "alternates", i);
    HS_ALWAYS_OR_UNREACHABLE(ok, "nosys.aou", "alternates", i);
    HS_SOMETIMES(ok, "nosys.sometimes", "alternates", i);
    HS_REACHABLE("nosys.reachable", "hit", i);
    HS_UNREACHABLE("nosys.unreachable", "hit", i);
    HS_WORK(1);
  }
  hs_thread("nosys", 1);
  syscall(SYS_exit, 0);
  return NULL;
}

int main(void) {
  pid_t pid = fork();
  if (pid == 0) {
    pthread_t t;
    if (pthread_create(&t, NULL, asserting_thread, NULL) != 0) _exit(91);
    pthread_join(t, NULL);
    _exit(0);
  }
  int status = 0;
  waitpid(pid, &status, 0);
  if (WIFEXITED(status) && WEXITSTATUS(status) == 0) {
    printf("ok   no system call during %d evaluations of every kind\n", EVALUATIONS);
    return 0;
  }
  printf("FAIL call sites made a system call or the filter could not be installed (status 0x%x)\n", status);
  return 1;
}
#endif
