/* seccomp_window.c -- build-record check B3 (Linux only, test-only: never pinned, never transferred).
**
** A fresh thread installs a seccomp-BPF filter that allows only exit, exit_group and rt_sigreturn, then runs the
** fixture's window many times (load the counter, spin for W = LANG_AT_SPIN rounds, store) and ends with a raw
** exit. Any other system call inside the window kills the process, and the parent sees a signal instead of a clean
** exit. It uses a seccomp-BPF filter, like sdks/c/tests/test_nosyscall.c, rather than SECCOMP_MODE_STRICT; strict mode would
** also allow read and write, which the window must not use either. It compiles as C and as C++ (the C++ fixtures are checked with the
** same file) and uses only the shared header's busy loop.
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
#include <stdint.h>
#include <stdio.h>
#include <sys/prctl.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>

#include "lang_at.h"

#define WINDOWS 20000u
#define FILTER_FAILED 90
#define THREAD_FAILED 91

static uint64_t counter;

static void allow_only_exit(void) {
  struct sock_filter prog[] = {
      BPF_STMT(BPF_LD | BPF_W | BPF_ABS, offsetof(struct seccomp_data, nr)),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_exit, 3, 0),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_exit_group, 2, 0),
      BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_rt_sigreturn, 1, 0),
      BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
      BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
  };
  struct sock_fprog fprog;
  fprog.len = (unsigned short)(sizeof prog / sizeof prog[0]);
  fprog.filter = prog;
  if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 || prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &fprog) != 0) {
    _exit(FILTER_FAILED); /* the filter could not be installed: do not claim a pass */
  }
}

static void *window_thread(void *arg) {
  (void)arg;
  allow_only_exit();
  for (unsigned i = 0; i < WINDOWS; i++) {
    uint64_t seen = __atomic_load_n(&counter, __ATOMIC_RELAXED);
    uint64_t pad = lang_at_spin(seen, LANG_AT_SPIN);
    __atomic_store_n(&counter, seen + 1u, __ATOMIC_RELAXED);
    lang_at_keep(0, pad);
  }
  syscall(SYS_exit, 0);
  return NULL;
}

int main(void) {
  pid_t pid = fork();
  if (pid == 0) {
    pthread_t t;
    if (pthread_create(&t, NULL, window_thread, NULL) != 0) _exit(THREAD_FAILED);
    pthread_join(t, NULL);
    _exit(0);
  }
  int status = 0;
  waitpid(pid, &status, 0);
  if (WIFEXITED(status) && WEXITSTATUS(status) == 0) {
    printf("ok   no system call during %u windows (load, spin %u, store)\n", WINDOWS, LANG_AT_SPIN);
    return 0;
  }
  printf("FAIL the window made a system call or the filter could not be installed (status 0x%x)\n", status);
  return 1;
}
#endif
