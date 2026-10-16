/* hairspring.c -- thread slots, process identity, startup and exit wiring for the Hairspring C SDK. */
#include <fcntl.h>
#include <pthread.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "hs_internal.h"

#define DEFAULT_PROC_TAG "p1"
#define ROOT_TAG_MAX 40 /* leaves room for ".f<n>" suffixes before a derived tag has to be hashed */
#define OVERFLOW_KEY "t#overflow"

struct hs_proc hs_proc = {.tag = DEFAULT_PROC_TAG, .fd = 2};
struct hs_slot hs_slots[HS_MAX_THREADS];
uint64_t hs_claims;
__thread struct hs_slot *hs__tls;

static int initialised;

/* ---- slots ---------------------------------------------------------------------------------------------------- */

size_t hs_slots_used(void) {
  uint64_t n = __atomic_load_n(&hs_claims, __ATOMIC_RELAXED);
  return n < HS_OVERFLOW_SLOT ? (size_t)n : HS_MAX_THREADS;
}

static void set_default_key(struct hs_slot *sl, uint64_t claim_order) {
  struct hs_buf b;
  hs_buf_init(&b, sl->key, HS_THREAD_KEY_MAX);
  if (claim_order == HS_OVERFLOW_SLOT) hs_put_str(&b, OVERFLOW_KEY);
  else {
    hs_put_str(&b, "t#");
    hs_put_u64(&b, claim_order + 1u);
  }
  sl->key[b.len] = 0;
  sl->is_stable = 0;
}

struct hs_slot *hs__claim(void) {
  if (!initialised) hs_init();
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
  if (sl->index == HS_OVERFLOW_SLOT || key_taken(sl, key)) return -1;
  memcpy(sl->key, key, b.len + 1u);
  sl->is_stable = 1;
  return 0;
}

/* ---- startup -------------------------------------------------------------------------------------------------- */

static void choose_tag(void) {
  const char *want = getenv("HAIRSPRING_PROC");
  if (!want || want[0] == 0) return;
  hs_ident_copy(hs_proc.tag, want, ROOT_TAG_MAX);
}

static void open_events(void) {
  const char *mode = getenv("HAIRSPRING_EVENTS");
  const char *path = getenv("HAIRSPRING_EVENTS_FILE");
  if (mode && strcmp(mode, "off") == 0) {
    hs_proc.fd = -1;
    return;
  }
  hs_proc.fd = 2;
  if (!path || path[0] == 0) return;
  int fd = open(path, O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC, 0644);
  if (fd >= 0) hs_proc.fd = fd;
  else hs_proc.dropped++; /* the file could not be opened: events fall back to stderr, and this is counted */
}

/* The tag of the n-th child: "<parent>.f<n>", or a hash of that when it would not fit in a tag. */
static void derive_child_tag(uint32_t n) {
  char next[HS_TAG_MAX + 1];
  struct hs_buf b;
  hs_buf_init(&b, next, HS_TAG_MAX);
  hs_put_str(&b, hs_proc.tag);
  hs_put_str(&b, ".f");
  hs_put_u64(&b, n);
  if (b.over) {
    hs_buf_init(&b, next, HS_TAG_MAX);
    hs_put_str(&b, "h");
    hs_put_hex64(&b, hs_ident_hash(hs_proc.tag, HS_TAG_MAX) ^ ((uint64_t)n * 0x9e3779b97f4a7c15ULL));
  }
  next[b.len] = 0;
  memcpy(hs_proc.tag, next, b.len + 1u);
}

static void atfork_prepare(void) { hs_proc.forks++; }

/* In the child: a new process with its own tag, empty counters and only the forking thread. */
static void atfork_child(void) {
  struct hs_slot *mine = hs__tls;
  char key[HS_THREAD_KEY_MAX + 1];
  uint64_t stable = 0;
  if (mine) {
    memcpy(key, mine->key, sizeof key);
    stable = mine->is_stable;
  }
  derive_child_tag(hs_proc.forks);
  hs_proc.forks = 0;
  hs_proc.dropped = 0;
  hs_proc.status = 0;
  memset(hs_slots, 0, sizeof hs_slots);
  hs_claims = 0;
  hs__tls = NULL;
  if (mine) {
    struct hs_slot *sl = hs__claim();
    memcpy(sl->key, key, sizeof key);
    sl->is_stable = stable;
  }
  hs_catalog_reset();
  hs_report_rearm();
  hs_emit_startup();
}

static void at_exit_report(void) { hs_report_now(HS_END_EXIT, hs_proc.status); }

void hs_init(void) {
  if (__atomic_exchange_n(&initialised, 1, __ATOMIC_ACQ_REL)) return;
  choose_tag();
  open_events();
  hs_proc.dropped += hs_catalog_build();
  const char *signals = getenv("HAIRSPRING_SIGNAL_REPORT");
  if (signals && strcmp(signals, "1") == 0) {
    hs_install_signal_handlers();
    hs_proc.signal_handlers = 1;
  }
  atexit(at_exit_report);
  pthread_atfork(atfork_prepare, NULL, atfork_child);
  hs_emit_startup();
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
