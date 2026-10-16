/* hairspring.c -- thread slots, startup and exit wiring for the Hairspring C SDK. */
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "hs_internal.h"

#define OVERFLOW_KEY "t#overflow"
#define HIGH_FD_FIRST 200 /* the private events descriptor lives well above the ones a program numbers itself */
#define HIGH_FD_SECOND 20 /* used when RLIMIT_NOFILE is too low for the first */

struct hs_proc hs_proc = {.tag = "p1", .fd = -1};
struct hs_slot hs_slots[HS_MAX_THREADS];
uint64_t hs_claims;
__thread struct hs_slot *hs__tls HS__TLS_MODEL;

static int initialised;
static int key_lock; /* serialises hs_thread (not a call site): the duplicate-key check and the key write */

/* ---- slots ---------------------------------------------------------------------------------------------------- */

size_t hs_slots_used(void) {
  uint64_t n = __atomic_load_n(&hs_claims, __ATOMIC_RELAXED);
  return n < HS_OVERFLOW_SLOT ? (size_t)n : HS_MAX_THREADS;
}

/* Write `key` into the slot so that key[0] becomes non-zero last: a reporter treats a slot as live only then. */
static void publish_key(struct hs_slot *sl, const char *key, size_t len) {
  memcpy(sl->key + 1, key + 1, len); /* includes the terminating NUL */
  __atomic_store_n(&sl->key[0], key[0], __ATOMIC_RELEASE);
}

static void set_default_key(struct hs_slot *sl, uint64_t claim_order) {
  char key[HS_THREAD_KEY_MAX + 1];
  struct hs_buf b;
  hs_buf_init(&b, key, HS_THREAD_KEY_MAX);
  if (claim_order == HS_OVERFLOW_SLOT) {
    hs_put_str(&b, OVERFLOW_KEY);
  } else {
    hs_put_str(&b, "t#");
    hs_put_u64(&b, claim_order + 1u);
  }
  key[b.len] = 0;
  sl->is_stable = 0;
  publish_key(sl, key, b.len);
}

struct hs_slot *hs__claim(void) {
  if (!initialised) hs_init(); /* a call site that runs before the constructor does the startup itself */
  uint64_t n = __atomic_fetch_add(&hs_claims, 1u, __ATOMIC_RELAXED);
  uint64_t index = n < HS_OVERFLOW_SLOT ? n : HS_OVERFLOW_SLOT;
  struct hs_slot *sl = &hs_slots[index];
  if (n <= HS_OVERFLOW_SLOT) {
    sl->index = index;
    set_default_key(sl, index);
  }
  if (n >= HS_OVERFLOW_SLOT) __atomic_fetch_add(&hs_proc.dropped, 1u, __ATOMIC_RELAXED); /* shares a slot */
  hs__tls = sl;
  return sl;
}

static void lock_keys(void) {
  while (__atomic_exchange_n(&key_lock, 1, __ATOMIC_ACQUIRE)) {
  }
}

static void unlock_keys(void) { __atomic_store_n(&key_lock, 0, __ATOMIC_RELEASE); }

static int key_taken(const struct hs_slot *self, const char *key) {
  size_t used = hs_slots_used();
  for (size_t i = 0; i < used; i++)
    if (&hs_slots[i] != self && strcmp(hs_slots[i].key, key) == 0) return 1;
  return 0;
}

int hs_thread(const char *label, unsigned index) {
  if (!label || label[0] == 0) return -1;
  char key[HS_THREAD_KEY_MAX + 1];
  struct hs_buf b;
  hs_buf_init(&b, key, HS_THREAD_KEY_MAX);
  for (size_t i = 0; label[i] && !b.over; i++) {
    char one = hs_ident_char((unsigned char)label[i]) ? label[i] : '_';
    hs_put_bytes(&b, &one, 1);
  }
  hs_put_str(&b, "-");
  hs_put_u64(&b, index);
  if (b.over) return -1;
  key[b.len] = 0;
  struct hs_slot *sl = hs__slot();
  if (sl->index == HS_OVERFLOW_SLOT) return -1;
  lock_keys();
  int taken = key_taken(sl, key);
  if (!taken) {
    publish_key(sl, key, b.len);
    sl->is_stable = 1;
  }
  unlock_keys();
  return taken ? -1 : 0;
}

/* ---- startup -------------------------------------------------------------------------------------------------- */

/* A copy of `fd` at a high number, close-on-exec, so the program closing or reusing a low descriptor (2 above all)
** cannot redirect the report into one of its own files. Returns -1 if `fd` is not open. If no high number is
** available the original is used and still exposed to that. */
static int private_copy(int fd) {
  int copy = fcntl(fd, F_DUPFD_CLOEXEC, HIGH_FD_FIRST);
  if (copy < 0) copy = fcntl(fd, F_DUPFD_CLOEXEC, HIGH_FD_SECOND);
  if (copy >= 0) return copy;
  return fcntl(fd, F_GETFD) >= 0 ? fd : -1;
}

static void open_events(void) {
  const char *mode = getenv("HAIRSPRING_EVENTS");
  const char *path = getenv("HAIRSPRING_EVENTS_FILE");
  hs_proc.fd = -1;
  if (mode && strcmp(mode, "off") == 0) return;
  if (path && path[0]) {
    int fd = open(path, O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC | O_NOFOLLOW, 0644);
    if (fd >= 0) {
      int high = private_copy(fd);
      if (high != fd && high >= 0) close(fd);
      hs_proc.fd = high >= 0 ? high : fd;
      return;
    }
    hs_proc.dropped++; /* the file could not be opened: events fall back to stderr, and this is counted */
  }
  hs_proc.fd = private_copy(2);
}

static void atfork_prepare(void) { hs_proc.forks++; }

/* In the child: a new process with its own tag, empty counters and only the forking thread. It announces itself
** (hello and catalog) when it reports, not now: if it execs an uninstrumented image it leaves nothing behind. */
static void atfork_child(void) {
  struct hs_slot *mine = hs__tls;
  char key[sizeof mine->key];
  uint64_t stable = 0;
  if (mine) {
    memcpy(key, mine->key, sizeof key);
    stable = mine->is_stable;
  }
  hs_identity_child(hs_proc.forks);
  hs_proc.forks = 0;
  hs_proc.dropped = 0;
  hs_proc.status = 0;
  memset(hs_slots, 0, sizeof hs_slots);
  hs_claims = 0;
  hs__tls = NULL;
  if (mine) {
    struct hs_slot *sl = hs__claim();
    publish_key(sl, key, strlen(key));
    sl->is_stable = stable;
  }
  hs_catalog_reset();
  hs_report_rearm();
  hs_proc.startup_pending = 1;
}

#if defined(__GLIBC__)
/* glibc hands exit handlers the real status, so a plain `return n` from main is reported as n. */
static void at_exit_report(int status, void *unused) {
  (void)unused;
  hs_report_now(HS_END_EXIT, status);
}
#define REGISTER_EXIT_HANDLER() on_exit(at_exit_report, NULL)
#else
/* Elsewhere an exit handler cannot see the status: `end.status` is 0 unless hs_exit() recorded one. It is advisory
** either way; the engine uses the OS wait status. */
static void at_exit_report(void) { hs_report_now(HS_END_EXIT, hs_proc.status); }
#define REGISTER_EXIT_HANDLER() atexit(at_exit_report)
#endif

void hs_init(void) {
  if (__atomic_exchange_n(&initialised, 1, __ATOMIC_ACQ_REL)) return;
  int saved_errno = errno;
  hs_identity_init();
  open_events();
  hs_proc.dropped += hs_catalog_build();
  const char *signals = getenv("HAIRSPRING_SIGNAL_REPORT");
  if (signals && strcmp(signals, "1") == 0 && hs_install_signal_handlers() == 0) hs_proc.signal_handlers = 1;
  REGISTER_EXIT_HANDLER();
  pthread_atfork(atfork_prepare, NULL, atfork_child);
  hs_emit_startup();
  errno = saved_errno;
}

__attribute__((constructor)) static void hs_constructor(void) { hs_init(); }

/* ---- exit ----------------------------------------------------------------------------------------------------- */

void hs_report(void) {
  hs_init();
  hs_report_now(HS_END_EXPLICIT, hs_proc.status);
}

void hs_exit(int status) {
  hs_proc.status = status;
  exit(status);
}
