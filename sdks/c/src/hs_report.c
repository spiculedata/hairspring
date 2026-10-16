/* hs_report.c -- writing lines (one write(2) each), the startup catalog, the exit/signal report, `eval` lines.
**
** Everything here runs at startup, at exit or in a fatal-signal handler, never at a call site. The report path uses
** no allocation and no stdio, so it can run from a signal handler. */
#include <errno.h>
#include <signal.h>
#include <string.h>
#include <unistd.h>

#include "hs_internal.h"

#define EINTR_RETRIES 16
#define ALT_STACK_BYTES (64 * 1024)
#define HS_EVAL_LINE_MAX 512 /* prefix, a 128-byte id, a kind name and the fixed members */

static char line_store[HS_LINE_MAX]; /* the one line buffer: startup and the guarded report are single-writer */
static int report_done;              /* set by the first report; hs_report_rearm clears it in a fork child */

void hs_test_set_sink(hs_sink_fn fn) { hs_proc.sink = fn; }

/* One write(2) per line. A write of at most PIPE_BUF bytes to a pipe is atomic (PIPE_BUF is 4096 on Linux and 512
** on macOS, where larger pipe writes may still interleave: see docs/c-sdk.md), and O_APPEND keeps each write to
** the events file contiguous. */
void hs_emit_line(const struct hs_buf *b) {
  if (hs_proc.sink) {
    hs_proc.sink(b->p, b->len);
    return;
  }
  if (hs_proc.fd < 0) return;
  int saved_errno = errno;
  ssize_t w = -1;
  for (int tries = 0; tries < EINTR_RETRIES; tries++) {
    w = write(hs_proc.fd, b->p, b->len);
    if (w >= 0 || errno != EINTR) break;
  }
  if (w != (ssize_t)b->len) __atomic_fetch_add(&hs_proc.dropped, 1, __ATOMIC_RELAXED);
  errno = saved_errno;
}

/* Build one line into the shared buffer and send it; a line that does not fit is counted, never truncated. */
static void send_built(int built, struct hs_buf *b) {
  if (built == 0) hs_emit_line(b);
  else __atomic_fetch_add(&hs_proc.dropped, 1, __ATOMIC_RELAXED);
}

static void fresh(struct hs_buf *b) { hs_buf_init(b, line_store, sizeof line_store); }

/* A catalog line goes out once per (id, kind): the canonical record, plus any record that disagrees on kind. */
static int wants_catalog_line(const struct hs_site *s) {
  return s->canon && (s->canon == s || s->canon->kind != s->kind);
}

void hs_emit_startup(void) {
  struct hs_buf b;
  fresh(&b);
  send_built(hs_line_hello(&b), &b);
  for (const struct hs_site *s = hs_sites_begin(); s < hs_sites_end(); s++) {
    if (!hs_site_live(s) || !wants_catalog_line(s)) continue;
    fresh(&b);
    send_built(hs_line_catalog(&b, s), &b);
  }
}

static void emit_asserts(void) {
  struct hs_buf b;
  for (const struct hs_site *s = hs_sites_begin(); s < hs_sites_end(); s++) {
    if (!hs_site_live(s) || s->canon != s) continue;
    fresh(&b);
    send_built(hs_line_assert(&b, s), &b);
  }
}

static void emit_threads(uint64_t *evals_out, uint64_t *work_out) {
  struct hs_buf b;
  uint64_t evals = 0, work = 0;
  size_t used = hs_slots_used();
  for (size_t i = 0; i < used; i++) {
    evals += __atomic_load_n(&hs_slots[i].evals, __ATOMIC_RELAXED);
    work += __atomic_load_n(&hs_slots[i].work, __ATOMIC_RELAXED);
  }
  fresh(&b);
  send_built(hs_line_work(&b, work), &b);
  for (size_t i = 0; i < used; i++) {
    fresh(&b);
    send_built(hs_line_thread(&b, &hs_slots[i]), &b);
  }
  *evals_out = evals;
  *work_out = work;
}

void hs_report_now(enum hs_end_reason reason, int64_t status) {
  if (__atomic_exchange_n(&report_done, 1, __ATOMIC_ACQ_REL)) return;
  hs_catalog_sum();
  emit_asserts();
  uint64_t evals = 0, work = 0;
  emit_threads(&evals, &work);
  struct hs_buf b;
  fresh(&b);
  send_built(hs_line_end(&b, reason, status, evals), &b);
}

void hs_report_rearm(void) { __atomic_store_n(&report_done, 0, __ATOMIC_RELEASE); }

/* ---- fatal signals (opt-in) ----------------------------------------------------------------------------------- */

static char alt_stack[ALT_STACK_BYTES];

static void on_fatal_signal(int sig) {
  hs_report_now(HS_END_SIGNAL, sig);
  raise(sig); /* SA_RESETHAND restored the default action, so this ends the process as it would have ended */
}

void hs_install_signal_handlers(void) {
  stack_t ss;
  memset(&ss, 0, sizeof ss);
  ss.ss_sp = alt_stack;
  ss.ss_size = sizeof alt_stack;
  sigaltstack(&ss, NULL);
  static const int signals[] = {SIGSEGV, SIGBUS, SIGABRT, SIGFPE, SIGILL, SIGTERM};
  for (size_t i = 0; i < sizeof signals / sizeof signals[0]; i++) {
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = on_fatal_signal;
    sa.sa_flags = SA_ONSTACK | SA_RESETHAND | SA_NODEFER;
    sigemptyset(&sa.sa_mask);
    sigaction(signals[i], &sa, NULL);
  }
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
  struct hs_buf b;
  char local[HS_EVAL_LINE_MAX]; /* its own buffer: eval lines may come from several threads at once */
  hs_buf_init(&b, local, sizeof local);
  if (hs_line_eval(&b, id, type, ok) != 0) return -1;
  hs_emit_line(&b);
  return 0;
}
