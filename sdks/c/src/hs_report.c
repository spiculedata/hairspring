/* hs_report.c -- writing lines (one write(2) each), the startup catalog, the exit/signal report, `eval` lines.
**
** Everything here runs at startup, at exit or in a fatal-signal handler, never at a call site. The report path uses
** no allocation and no stdio, so it can run from a signal handler. */
#include <errno.h>
#include <sched.h>
#include <signal.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "hs_internal.h"

#define RETRY_LIMIT 1000 /* EINTR and EAGAIN retries per line */
#define ALT_STACK_BYTES (64 * 1024)
#define HS_EVAL_LINE_MAX 512 /* prefix, a 128-byte id, a kind name and the fixed members */

static char line_store[HS_LINE_MAX]; /* the one line buffer: startup and the guarded report are single-writer */
static int report_done;              /* set by the first report; hs_report_rearm clears it in a fork child */

void hs_test_set_sink(hs_sink_fn fn) { hs_proc.sink = fn; }

/* The events descriptor can be lost to the program (dup2 onto it, or closing everything) and then name some other
** file. Remember what it referred to at startup; the report checks once, with one fstat, before writing. */
static dev_t events_dev;
static ino_t events_ino;

/* Called before the descriptor is published in hs_proc.fd (a release store), so a thread that sees the descriptor
** also sees the identity it is compared with. Returns `fd`, or -1 if it cannot be identified. */
int hs_events_fd_remember(int fd) {
  struct stat st;
  if (fd < 0 || fstat(fd, &st) != 0) return -1;
  events_dev = st.st_dev;
  events_ino = st.st_ino;
  return fd;
}

/* Checked before the report and before every eval line. Once it fails the descriptor is given up for good, so no
** later line can follow it either. */
static void verify_events_fd(void) {
  int fd = __atomic_load_n(&hs_proc.fd, __ATOMIC_ACQUIRE);
  if (fd < 0 || hs_proc.sink) return;
  int saved_errno = errno;
  struct stat st;
  int same = fstat(fd, &st) == 0 && st.st_dev == events_dev && st.st_ino == events_ino;
  errno = saved_errno;
  if (same) return;
  __atomic_fetch_add(&hs_proc.dropped, 1, __ATOMIC_RELAXED); /* nothing is written: the descriptor leads elsewhere */
  __atomic_store_n(&hs_proc.fd, -1, __ATOMIC_RELEASE);
}

/* ---- the write path ------------------------------------------------------------------------------------------- */

struct out_guard {
  sigset_t old;
  int saw_epipe;
};

/* A report to a closed pipe must not kill the process with SIGPIPE: block it while writing. */
static void out_begin(struct out_guard *g) {
  sigset_t pipe_only;
  sigemptyset(&pipe_only);
  sigaddset(&pipe_only, SIGPIPE);
  pthread_sigmask(SIG_BLOCK, &pipe_only, &g->old);
  g->saw_epipe = 0;
}

/* Restore the mask. If our own write raised a SIGPIPE, consume it first so it is not delivered on unblocking.
** sigwait is not on POSIX's async-signal-safe list; this function is never reached from the signal path (the fatal
** handler leaves the mask alone and returns without calling it). */
static void out_end(struct out_guard *g) {
  if (g->saw_epipe && !sigismember(&g->old, SIGPIPE)) {
    sigset_t pipe_only, pending;
    sigemptyset(&pipe_only);
    sigaddset(&pipe_only, SIGPIPE);
    sigpending(&pending);
    int sig = 0;
    if (sigismember(&pending, SIGPIPE)) sigwait(&pipe_only, &sig);
  }
  pthread_sigmask(SIG_SETMASK, &g->old, NULL);
}

static __thread struct out_guard *current_guard; /* per thread: SIGPIPE from a write is delivered to that thread */

/* Write all of [p, p+len). Returns 0, or the number of bytes written before a failure, negated, minus one. */
static long write_all(int fd, const char *p, size_t len) {
  size_t done = 0;
  int retries = 0;
  while (done < len) {
    ssize_t w = write(fd, p + done, len - done);
    if (w > 0) {
      done += (size_t)w;
      continue;
    }
    if (w < 0 && retries++ < RETRY_LIMIT) {
      if (errno == EINTR) continue;
      if (errno == EAGAIN || errno == EWOULDBLOCK) {
        sched_yield();
        continue;
      }
    }
    if (w < 0 && errno == EPIPE && current_guard) current_guard->saw_epipe = 1;
    return -(long)done - 1;
  }
  return 0;
}

/* One write(2) per line in the normal case. A short write (a signal, a full pipe) is completed; a line that still
** fails after part of it went out is closed with a newline so the next line does not glue onto the fragment. */
void hs_emit_line(const struct hs_buf *b) {
  if (hs_proc.sink) {
    hs_proc.sink(b->p, b->len);
    return;
  }
  int fd = __atomic_load_n(&hs_proc.fd, __ATOMIC_ACQUIRE);
  if (fd < 0) return;
  int saved_errno = errno;
  long failed = write_all(fd, b->p, b->len);
  if (failed != 0) {
    __atomic_fetch_add(&hs_proc.dropped, 1, __ATOMIC_RELAXED);
    if (failed < -1) (void)!write(fd, "\n", 1);
  }
  errno = saved_errno;
}

/* Build one line into the shared buffer and send it; a line that does not fit is counted, never truncated. */
static void send_built(int built, struct hs_buf *b) {
  if (built == 0) hs_emit_line(b);
  else __atomic_fetch_add(&hs_proc.dropped, 1, __ATOMIC_RELAXED);
}

static void fresh(struct hs_buf *b) { hs_buf_init(b, line_store, sizeof line_store); }

/* ---- startup catalog ------------------------------------------------------------------------------------------ */

void hs_emit_startup(void) {
  struct hs_buf b;
  fresh(&b);
  send_built(hs_line_hello(&b), &b);
  for (const struct hs_site *s = hs_sites_begin(); s < hs_sites_end(); s++) {
    if (!hs_site_live(s) || !s->canon || !(s->flags & HS_SITE_EMIT)) continue;
    fresh(&b);
    send_built(hs_line_catalog(&b, s), &b);
  }
}

/* ---- the report ----------------------------------------------------------------------------------------------- */

static void emit_asserts(void) {
  struct hs_buf b;
  for (const struct hs_site *s = hs_sites_begin(); s < hs_sites_end(); s++) {
    if (!hs_site_live(s) || s->canon != s) continue;
    fresh(&b);
    send_built(hs_line_assert(&b, s), &b);
  }
}

/* Read every thread's totals once. A slot counts only once its key is published (claims are lock-free, so a slot
** can be claimed a moment before its key is written); the thread lines and `end` all come from this one reading. */
static size_t snapshot_threads(struct hs_thread_totals *out) {
  size_t n = 0;
  size_t used = hs_slots_used();
  for (size_t i = 0; i < used; i++) {
    const struct hs_slot *sl = &hs_slots[i];
    if (__atomic_load_n(&sl->key[0], __ATOMIC_ACQUIRE) == 0) continue;
    out[n].slot = sl;
    out[n].evals = __atomic_load_n(&sl->evals, __ATOMIC_RELAXED);
    out[n].work = __atomic_load_n(&sl->work, __ATOMIC_RELAXED);
    out[n].tape_consumed = __atomic_load_n(&sl->tape_consumed, __ATOMIC_RELAXED);
    out[n].is_stable = __atomic_load_n(&sl->is_stable, __ATOMIC_RELAXED);
    n++;
  }
  return n;
}

static void report_body(enum hs_end_reason reason, int64_t status) {
  verify_events_fd();
  if (hs_proc.startup_pending) { /* a fork child announces itself only now, so an exec into another image leaves nothing */
    hs_proc.startup_pending = 0;
    hs_emit_startup();
  }
  hs_catalog_sum();
  emit_asserts();
  static struct hs_thread_totals totals[HS_MAX_THREADS]; /* off the stack; single-writer behind report_done */
  size_t n = snapshot_threads(totals);
  uint64_t evals = 0, work = 0;
  for (size_t i = 0; i < n; i++) {
    evals += totals[i].evals;
    work += totals[i].work;
  }
  (void)hs_audit_run(reason, n); /* before `end`, whose `dropped` it may raise */
  struct hs_buf b;
  fresh(&b);
  send_built(hs_line_work(&b, work), &b);
  for (size_t i = 0; i < n; i++) {
    fresh(&b);
    send_built(hs_line_thread(&b, &totals[i]), &b);
  }
  fresh(&b);
  send_built(hs_line_end(&b, reason, status, evals, n), &b);
}

void hs_report_now(enum hs_end_reason reason, int64_t status) {
  if (__atomic_exchange_n(&report_done, 1, __ATOMIC_ACQ_REL)) return;
  int saved_errno = errno; /* the report is not the program's business: whatever it did, errno comes back */
  struct out_guard guard;
  out_begin(&guard);
  current_guard = &guard;
  report_body(reason, status);
  current_guard = NULL;
  /* After a fatal signal the process is about to die: keep SIGPIPE blocked rather than risk dying of it first. */
  if (reason != HS_END_SIGNAL) out_end(&guard);
  errno = saved_errno;
}

void hs_report_rearm(void) { __atomic_store_n(&report_done, 0, __ATOMIC_RELEASE); }

/* ---- fatal signals (opt-in) ----------------------------------------------------------------------------------- */

static char alt_stack[ALT_STACK_BYTES];

static void on_fatal_signal(int sig) {
  hs_report_now(HS_END_SIGNAL, sig);
  raise(sig); /* SA_RESETHAND restored the default action, so this ends the process as it would have ended */
}

/* True when `sig` still has the default action, so installing ours replaces nothing: an inherited SIG_IGN (from
** `trap '' TERM`, nohup) or a handler another runtime or a sanitizer put there stays. */
static int has_default_action(int sig) {
  struct sigaction old;
  /* sa_handler and sa_sigaction share storage, so this covers SA_SIGINFO handlers too. Flags alone mean nothing:
  ** macOS keeps SA_SIGINFO on a SIG_DFL action across exec when the parent (a Rust runtime, say) had a handler. */
  return sigaction(sig, NULL, &old) == 0 && old.sa_handler == SIG_DFL;
}

int hs_install_signal_handlers(void) {
  int failures = 0;
  stack_t ss;
  memset(&ss, 0, sizeof ss);
  ss.ss_sp = alt_stack;
  ss.ss_size = sizeof alt_stack;
  if (sigaltstack(&ss, NULL) != 0) failures++;
  static const int signals[] = {SIGSEGV, SIGBUS, SIGABRT, SIGFPE, SIGILL, SIGTERM};
  for (size_t i = 0; i < sizeof signals / sizeof signals[0]; i++) {
    if (!has_default_action(signals[i])) { /* counted like a failed install: the report cannot cover this signal */
      failures++;
      continue;
    }
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = on_fatal_signal;
    sa.sa_flags = SA_ONSTACK | SA_RESETHAND | SA_NODEFER;
    sigemptyset(&sa.sa_mask);
    if (sigaction(signals[i], &sa, NULL) != 0) failures++;
  }
  return failures;
}

/* ---- eval lines (perturbing) ---------------------------------------------------------------------------------- */

static int known_type(const char *type) {
  static const char *const kinds[] = {"always", "always_or_unreachable", "sometimes", "reachable", "unreachable"};
  for (size_t i = 0; i < sizeof kinds / sizeof kinds[0]; i++)
    if (strcmp(type, kinds[i]) == 0) return 1;
  return 0;
}

int hs_eval_line(const char *id, const char *type, int ok) {
  if (!id || !type || !known_type(type) || id[0] == 0) return -1;
  size_t n = 0;
  for (; id[n]; n++)
    if (n >= HS_ID_MAX || !hs_ident_char((unsigned char)id[n])) return -1;
  hs_init();
  verify_events_fd();
  struct hs_buf b;
  char local[HS_EVAL_LINE_MAX]; /* its own buffer: eval lines may come from several threads at once */
  hs_buf_init(&b, local, sizeof local);
  if (hs_line_eval(&b, id, type, ok) != 0) return -1;
  struct out_guard guard;
  out_begin(&guard);
  current_guard = &guard;
  hs_emit_line(&b);
  current_guard = NULL;
  out_end(&guard);
  return 0;
}
