/* hs_catalog.c -- the call-site records collected by the linker, and the per-id merge over them. */
#include <string.h>

#include "hs_internal.h"

/* Mach-O has no __start/__stop symbols; ld64 synthesises these for any existing section. ELF (GNU ld, lld, gold)
** synthesises __start_/__stop_ for a section whose name is a C identifier. Both need the section to exist, so the
** SDK always plants one sentinel record (kind HS_KIND_NONE) and one empty string in each section. */
#if defined(__APPLE__)
extern struct hs_site hs_sec_start __asm("section$start$__DATA$__hs_catalog");
extern struct hs_site hs_sec_stop __asm("section$end$__DATA$__hs_catalog");
extern const char hs_str_start __asm("section$start$__DATA$__hs_catstr");
extern const char hs_str_stop __asm("section$end$__DATA$__hs_catstr");
#define SEC_START (&hs_sec_start)
#define SEC_STOP (&hs_sec_stop)
#define STR_START (&hs_str_start)
#define STR_STOP (&hs_str_stop)
#else
extern struct hs_site __start_hs_catalog_v1[] __attribute__((weak));
extern struct hs_site __stop_hs_catalog_v1[] __attribute__((weak));
extern const char __start_hs_catalog_str_v1[] __attribute__((weak));
extern const char __stop_hs_catalog_str_v1[] __attribute__((weak));
#define SEC_START (__start_hs_catalog_v1)
#define SEC_STOP (__stop_hs_catalog_v1)
#define STR_START (__start_hs_catalog_str_v1)
#define STR_STOP (__stop_hs_catalog_str_v1)
#endif

static struct hs_site hs_sentinel __attribute__((used, section(HS__SECTION))) HS__RETAIN = {0};
static const char hs_str_sentinel[] __attribute__((used, section(HS__STR_SECTION))) HS__RETAIN = "";

struct hs_site *hs_sites_begin(void) { return SEC_START; }
struct hs_site *hs_sites_end(void) { return SEC_STOP; }
const char *hs_offline_begin(void) { return STR_START; }
const char *hs_offline_end(void) { return STR_STOP; }

#define TABLE_SIZE 16384u /* open addressing; a power of two comfortably above 2 * HS_MAX_IDS */
#define KIND_SHIFT 56
#define HASH_MIX 0x9e3779b97f4a7c15ULL

static struct hs_site *table[TABLE_SIZE];

static struct hs_site *lookup(const struct hs_site *s, size_t *free_slot) {
  size_t i = (size_t)(s->id_hash & (TABLE_SIZE - 1u));
  for (size_t probes = 0; probes < TABLE_SIZE; probes++, i = (i + 1u) & (TABLE_SIZE - 1u)) {
    struct hs_site *t = table[i];
    if (!t) {
      *free_slot = i;
      return NULL;
    }
    if (t->id_hash == s->id_hash && hs_ident_equal(t->id, s->id, HS_ID_MAX)) return t;
  }
  *free_slot = TABLE_SIZE; /* full: cannot happen below HS_MAX_IDS entries */
  return NULL;
}

uint64_t hs_catalog_build(void) {
  memset(table, 0, sizeof table);
  uint64_t ids = 0, excess = 0, hash = 0;
  for (struct hs_site *s = hs_sites_begin(); s < hs_sites_end(); s++) {
    if (!hs_site_live(s)) continue;
    s->id_hash = hs_ident_hash(s->id, HS_ID_MAX);
    size_t free_slot = 0;
    struct hs_site *found = lookup(s, &free_slot);
    if (found) {
      s->canon = found;
    } else if (ids < HS_MAX_IDS && free_slot < TABLE_SIZE) {
      table[free_slot] = s;
      s->canon = s;
      ids++;
      hash += (s->id_hash ^ ((uint64_t)s->kind << KIND_SHIFT)) * HASH_MIX;
    } else {
      s->canon = NULL;
      excess++;
    }
  }
  hs_proc.n_ids = ids;
  hs_proc.catalog_hash = hash;
  return excess;
}

static const struct hs_witness *witness_if_set(const struct hs_witness *w) { return w->ord != 0 ? w : NULL; }

void hs_catalog_sum(void) {
  for (struct hs_site *s = hs_sites_begin(); s < hs_sites_end(); s++) {
    if (!hs_site_live(s)) continue;
    s->sum_trues = __atomic_load_n(&s->trues, __ATOMIC_RELAXED);
    s->sum_falses = __atomic_load_n(&s->falses, __ATOMIC_RELAXED);
    s->pick_true = s->sum_trues ? witness_if_set(&s->first_true) : NULL;
    s->pick_false = s->sum_falses ? witness_if_set(&s->first_false) : NULL;
  }
  for (struct hs_site *s = hs_sites_begin(); s < hs_sites_end(); s++) {
    if (!hs_site_live(s) || !s->canon || s->canon == s) continue;
    struct hs_site *c = s->canon;
    c->sum_trues += __atomic_load_n(&s->trues, __ATOMIC_RELAXED);
    c->sum_falses += __atomic_load_n(&s->falses, __ATOMIC_RELAXED);
    if (!c->pick_true && s->trues) c->pick_true = witness_if_set(&s->first_true);
    if (!c->pick_false && s->falses) c->pick_false = witness_if_set(&s->first_false);
  }
}

void hs_catalog_reset(void) {
  const struct hs_witness none = {0, 0, {0, 0, 0, 0}};
  for (struct hs_site *s = hs_sites_begin(); s < hs_sites_end(); s++) {
    if (!hs_site_live(s)) continue;
    s->trues = 0;
    s->falses = 0;
    s->first_true = none;
    s->first_false = none;
  }
}
