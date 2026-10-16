/* hs_fmt.h -- allocation-free, async-signal-safe line formatting for the Hairspring C SDK. */
#ifndef HS_FMT_H
#define HS_FMT_H

#include <stddef.h>
#include <stdint.h>

#define HS_LINE_MAX 4096 /* one protocol line plus its newline: PIPE_BUF on Linux, but 512 on macOS (see docs/c-sdk.md, pipe atomicity) */
#define HS_NUM_WIDTH 20  /* counters are right-aligned in this many characters */
#define HS_ID_MAX 128
#define HS_THREAD_KEY_MAX 64
#define HS_MSG_MAX 512
#define HS_FILE_MAX 256

/* A bounded output buffer. Once `over` is set every later put is ignored, so the caller checks it once. */
struct hs_buf {
  char *p;
  size_t len;
  size_t cap;
  int over;
};

void hs_buf_init(struct hs_buf *b, char *storage, size_t cap);
void hs_put_str(struct hs_buf *b, const char *s);
void hs_put_bytes(struct hs_buf *b, const char *s, size_t n);
void hs_put_pad(struct hs_buf *b, size_t n);     /* n spaces */
void hs_put_u64_w(struct hs_buf *b, uint64_t v); /* right-aligned in HS_NUM_WIDTH */
void hs_put_i64_w(struct hs_buf *b, int64_t v);  /* right-aligned in HS_NUM_WIDTH */
void hs_put_i64(struct hs_buf *b, int64_t v);    /* unpadded */
void hs_put_u64(struct hs_buf *b, uint64_t v);   /* unpadded */
void hs_put_hex64(struct hs_buf *b, uint64_t v); /* 16 lowercase digits */

/* A quoted JSON string of at most `max` source bytes. Only printable ASCII survives: '"' and '\' are escaped,
** every other byte becomes '?'. This keeps free text inside the protocol's string rules without UTF-8 decoding. */
void hs_put_json_text(struct hs_buf *b, const char *s, size_t max);
/* A quoted id or process tag: bytes outside [A-Za-z0-9._:/-] become '_', at most `max` bytes. */
void hs_put_ident(struct hs_buf *b, const char *s, size_t max);

/* A quoted thread key: like an id, but '#' is allowed (claim-order keys are "t#<n>"). */
void hs_put_thread_key(struct hs_buf *b, const char *s, size_t max);

/* The same, followed by spaces up to 64 key bytes, so a line's length does not depend on which key a slot got. */
void hs_put_thread_key_padded(struct hs_buf *b, const char *s);

int hs_ident_char(unsigned char c);
/* Compare two ids as they would be sent (after replacement and truncation). */
int hs_ident_equal(const char *a, const char *b, size_t max);
uint64_t hs_ident_hash(const char *s, size_t max); /* FNV-1a over the sent form */
/* Copy the sent form of `s` into out (cap >= max+1) and NUL-terminate; returns its length. */
size_t hs_ident_copy(char *out, const char *s, size_t max);

#endif
