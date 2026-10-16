/* hs_identity.c -- the process tag, and how it survives fork, exec and spawn.
**
** The tag lives in the environment as HAIRSPRING_PROC (with the owning pid in HAIRSPRING_PROC_PID). Both variables
** point at static buffers handed to putenv, so a fork child can rewrite them in place, without allocating, and any
** image it later execs inherits a tag that no other process uses.
**
**  - root process: HAIRSPRING_PROC if the harness set it, else "p1";
**  - fork child:   "<parent>.f<n>", written into the environment by the atfork handler;
**  - spawned child (posix_spawn, system(): no fork handler ran): its inherited HAIRSPRING_PROC_PID is not its own
**    pid, so the inherited tag belongs to an ancestor and the child takes "<inherited>.s<pid>".
** Not detectable: an image that execs another image without forking keeps its own tag (both send the same one).
*/
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "hs_internal.h"

#define ENV_TAG "HAIRSPRING_PROC="
#define ENV_PID "HAIRSPRING_PROC_PID="
#define DEFAULT_TAG "p1"
#define HASH_PREFIX_MAX 40 /* readable part kept when a derived tag would pass 64 bytes */
#define HASH_MIX 0x9e3779b97f4a7c15ULL

static char env_tag[sizeof ENV_TAG + HS_TAG_MAX] = ENV_TAG DEFAULT_TAG;
static char env_pid[sizeof ENV_PID + HS_NUM_WIDTH] = ENV_PID "0";

static int tag_valid(const char *t) {
  size_t n = 0;
  for (; t[n]; n++)
    if (n >= HS_TAG_MAX || !hs_ident_char((unsigned char)t[n])) return 0;
  return n > 0 && strcmp(t, "_eval") != 0; /* "_eval" is reserved for eval lines */
}

/* base + "." + letter + n, or "<first 40 bytes of base>.h<hash>" when that would not fit in a tag. */
static void derive(char *out, const char *base, char letter, uint64_t n) {
  struct hs_buf b;
  hs_buf_init(&b, out, HS_TAG_MAX);
  hs_put_str(&b, base);
  hs_put_bytes(&b, ".", 1);
  hs_put_bytes(&b, &letter, 1);
  hs_put_u64(&b, n);
  if (b.over) {
    hs_buf_init(&b, out, HS_TAG_MAX);
    size_t keep = strlen(base) < HASH_PREFIX_MAX ? strlen(base) : HASH_PREFIX_MAX;
    hs_put_bytes(&b, base, keep);
    hs_put_str(&b, ".h");
    hs_put_hex64(&b, hs_ident_hash(base, HS_TAG_MAX) ^ (n * HASH_MIX) ^ (uint64_t)(unsigned char)letter);
  }
  out[b.len] = 0;
}

static void export_env(void) {
  struct hs_buf b;
  hs_buf_init(&b, env_tag + sizeof ENV_TAG - 1, HS_TAG_MAX);
  hs_put_str(&b, hs_proc.tag);
  env_tag[sizeof ENV_TAG - 1 + b.len] = 0;
  hs_buf_init(&b, env_pid + sizeof ENV_PID - 1, HS_NUM_WIDTH);
  hs_put_u64(&b, (uint64_t)getpid());
  env_pid[sizeof ENV_PID - 1 + b.len] = 0;
}

void hs_identity_init(void) {
  const char *inherited = getenv("HAIRSPRING_PROC");
  const char *owner = getenv("HAIRSPRING_PROC_PID");
  memcpy(hs_proc.tag, DEFAULT_TAG, sizeof DEFAULT_TAG);
  if (inherited && inherited[0]) {
    if (!tag_valid(inherited)) {
      hs_proc.dropped++; /* a bad tag from the harness is refused, not silently shortened */
    } else {
      memcpy(hs_proc.tag, inherited, strlen(inherited) + 1u);
      char *end = NULL;
      unsigned long long pid = owner ? strtoull(owner, &end, 10) : 0;
      int has_owner = owner && owner[0] && end && *end == 0;
      if (owner && (!has_owner || pid != (unsigned long long)getpid())) {
        char spawned[HS_TAG_MAX + 1];
        derive(spawned, inherited, 's', (uint64_t)getpid());
        memcpy(hs_proc.tag, spawned, strlen(spawned) + 1u);
      }
    }
  }
  export_env();
  putenv(env_tag);
  putenv(env_pid);
}

void hs_identity_child(uint32_t n) {
  char next[HS_TAG_MAX + 1];
  derive(next, hs_proc.tag, 'f', n);
  memcpy(hs_proc.tag, next, strlen(next) + 1u);
  export_env();
}
