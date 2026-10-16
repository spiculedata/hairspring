/* hs_fmt.c -- see hs_fmt.h. No libc calls: usable from a signal handler. */
#include "hs_fmt.h"

#define FNV_OFFSET 0xcbf29ce484222325ULL
#define FNV_PRIME 0x100000001b3ULL

void hs_buf_init(struct hs_buf *b, char *storage, size_t cap) {
  b->p = storage;
  b->len = 0;
  b->cap = cap;
  b->over = 0;
}

static void put_char(struct hs_buf *b, char c) {
  if (b->over) return;
  if (b->len >= b->cap) {
    b->over = 1;
    return;
  }
  b->p[b->len++] = c;
}

void hs_put_bytes(struct hs_buf *b, const char *s, size_t n) {
  for (size_t i = 0; i < n; i++) put_char(b, s[i]);
}

void hs_put_str(struct hs_buf *b, const char *s) {
  for (; *s; s++) put_char(b, *s);
}

void hs_put_pad(struct hs_buf *b, size_t n) {
  for (size_t i = 0; i < n; i++) put_char(b, ' ');
}

/* Write v into the tail of `digits` (HS_NUM_WIDTH bytes) and return the first used index. */
static size_t digits_of(uint64_t v, char *digits) {
  size_t i = HS_NUM_WIDTH;
  do {
    digits[--i] = (char)('0' + (v % 10u));
    v /= 10u;
  } while (v != 0);
  return i;
}

void hs_put_u64(struct hs_buf *b, uint64_t v) {
  char d[HS_NUM_WIDTH];
  size_t i = digits_of(v, d);
  hs_put_bytes(b, d + i, HS_NUM_WIDTH - i);
}

void hs_put_u64_w(struct hs_buf *b, uint64_t v) {
  char d[HS_NUM_WIDTH];
  size_t i = digits_of(v, d);
  hs_put_pad(b, i);
  hs_put_bytes(b, d + i, HS_NUM_WIDTH - i);
}

/* |v| as unsigned, correct for INT64_MIN. */
static uint64_t magnitude(int64_t v) { return v < 0 ? (uint64_t)0 - (uint64_t)v : (uint64_t)v; }

void hs_put_i64(struct hs_buf *b, int64_t v) {
  if (v < 0) put_char(b, '-');
  hs_put_u64(b, magnitude(v));
}

void hs_put_i64_w(struct hs_buf *b, int64_t v) {
  char d[HS_NUM_WIDTH];
  size_t i = digits_of(magnitude(v), d);
  size_t used = HS_NUM_WIDTH - i + (v < 0 ? 1u : 0u);
  hs_put_pad(b, HS_NUM_WIDTH - used);
  if (v < 0) put_char(b, '-');
  hs_put_bytes(b, d + i, HS_NUM_WIDTH - i);
}

void hs_put_hex64(struct hs_buf *b, uint64_t v) {
  for (int shift = 60; shift >= 0; shift -= 4) {
    unsigned nib = (unsigned)((v >> shift) & 0xfu);
    put_char(b, (char)(nib < 10u ? '0' + nib : 'a' + (nib - 10u)));
  }
}

void hs_put_json_text(struct hs_buf *b, const char *s, size_t max) {
  put_char(b, '"');
  for (size_t i = 0; i < max && s[i]; i++) {
    unsigned char c = (unsigned char)s[i];
    if (c == '"' || c == '\\') {
      put_char(b, '\\');
      put_char(b, (char)c);
    } else if (c >= 0x20 && c < 0x7f) {
      put_char(b, (char)c);
    } else {
      put_char(b, '?');
    }
  }
  put_char(b, '"');
}

int hs_ident_char(unsigned char c) {
  return (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '.' || c == '_' ||
         c == ':' || c == '/' || c == '-';
}

static char sent_char(unsigned char c) { return hs_ident_char(c) ? (char)c : '_'; }

void hs_put_ident(struct hs_buf *b, const char *s, size_t max) {
  put_char(b, '"');
  for (size_t i = 0; i < max && s[i]; i++) put_char(b, sent_char((unsigned char)s[i]));
  put_char(b, '"');
}

void hs_put_thread_key(struct hs_buf *b, const char *s, size_t max) {
  put_char(b, '"');
  for (size_t i = 0; i < max && s[i]; i++) put_char(b, s[i] == '#' ? '#' : sent_char((unsigned char)s[i]));
  put_char(b, '"');
}

void hs_put_thread_key_padded(struct hs_buf *b, const char *s) {
  size_t start = b->len;
  hs_put_thread_key(b, s, HS_THREAD_KEY_MAX);
  size_t used = b->len - start - 2u; /* the quotes */
  hs_put_pad(b, used < HS_THREAD_KEY_MAX ? HS_THREAD_KEY_MAX - used : 0u);
}

int hs_ident_equal(const char *a, const char *b, size_t max) {
  for (size_t i = 0; i < max; i++) {
    if (a[i] == 0 || b[i] == 0) return a[i] == b[i];
    if (sent_char((unsigned char)a[i]) != sent_char((unsigned char)b[i])) return 0;
  }
  return 1;
}

uint64_t hs_ident_hash(const char *s, size_t max) {
  uint64_t h = FNV_OFFSET;
  for (size_t i = 0; i < max && s[i]; i++) h = (h ^ (unsigned char)sent_char((unsigned char)s[i])) * FNV_PRIME;
  return h;
}

size_t hs_ident_copy(char *out, const char *s, size_t max) {
  size_t i = 0;
  for (; i < max && s[i]; i++) out[i] = sent_char((unsigned char)s[i]);
  out[i] = 0;
  return i;
}
