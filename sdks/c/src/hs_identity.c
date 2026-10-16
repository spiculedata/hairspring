/* hs_identity.c -- the process tag, and how it survives fork, exec and spawn.
**
** The tag lives in the environment as HAIRSPRING_PROC, with the owning pid and two flags in HAIRSPRING_PROC_PID
** ("<pid>", "<pid>:a", "<pid>:u", "<pid>:au"; a = the tag has been announced with a hello, u = the tag is not stable
** across runs). Both variables point at static buffers handed to putenv, so a fork child can rewrite them in place,
** without allocating, and any image it later execs inherits a tag that no other process uses.
**
**  - root process: HAIRSPRING_PROC if the harness set it, else "p1";
**  - fork child:   "<parent>.f<n>", written into the environment by the atfork handler (not announced yet);
**  - spawned child (posix_spawn, system(): no fork handler ran): its inherited HAIRSPRING_PROC_PID is not its own
**    pid, so the inherited tag belongs to an ancestor and the child takes "<inherited>.s<pid>", flagged unstable
**    (a pid differs between runs outside Hermit);
**  - exec without fork: the inherited pid IS its own and the tag was announced, so the new image takes
**    "<tag>.x1" (the old image is gone and will never send `end`).
** Limits: if the program replaces or clears `environ`, the variables are gone, children start as "p1" and the trial is
** degraded. The backend must strip a stray HAIRSPRING_PROC and HAIRSPRING_PROC_PID from the guest environment.
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
#define PID_FIELD_MAX (HS_NUM_WIDTH + 4) /* digits, ':', two flags */

static char env_tag[sizeof ENV_TAG + HS_TAG_MAX] = ENV_TAG DEFAULT_TAG;
static char env_pid[sizeof ENV_PID + PID_FIELD_MAX] = ENV_PID "0";
static int announced; /* this process has sent its hello under the current tag */

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

static void adopt(const char *tag) { memcpy(hs_proc.tag, tag, strlen(tag) + 1u); }

static void export_env(void) {
  struct hs_buf b;
  hs_buf_init(&b, env_tag + sizeof ENV_TAG - 1, HS_TAG_MAX);
  hs_put_str(&b, hs_proc.tag);
  env_tag[sizeof ENV_TAG - 1 + b.len] = 0;
  hs_buf_init(&b, env_pid + sizeof ENV_PID - 1, PID_FIELD_MAX);
  hs_put_u64(&b, (uint64_t)getpid());
  if (announced || !hs_proc.tag_stable) {
    hs_put_str(&b, ":");
    if (announced) hs_put_str(&b, "a");
    if (!hs_proc.tag_stable) hs_put_str(&b, "u");
  }
  env_pid[sizeof ENV_PID - 1 + b.len] = 0;
}

struct owner {
  int present;
  int valid;
  unsigned long long pid;
  int announced;
  int unstable;
};

static struct owner parse_owner(const char *text) {
  struct owner o = {0, 0, 0, 0, 0};
  if (!text) return o;
  o.present = 1;
  char *end = NULL;
  o.pid = strtoull(text, &end, 10);
  if (end == text) return o;
  o.valid = 1;
  if (*end == ':') {
    for (end++; *end; end++) {
      if (*end == 'a') o.announced = 1;
      else if (*end == 'u') o.unstable = 1;
      else o.valid = 0;
    }
  }
  return o;
}

void hs_identity_init(void) {
  const char *inherited = getenv("HAIRSPRING_PROC");
  struct owner owner = parse_owner(getenv("HAIRSPRING_PROC_PID"));
  adopt(DEFAULT_TAG);
  hs_proc.tag_stable = 1;
  if (inherited && inherited[0]) {
    if (!tag_valid(inherited)) {
      hs_proc.dropped++; /* a bad tag from the harness is refused, not silently shortened */
    } else {
      adopt(inherited);
      char next[HS_TAG_MAX + 1];
      if (owner.present && !(owner.valid && owner.pid == (unsigned long long)getpid())) {
        derive(next, inherited, 's', (uint64_t)getpid());
        adopt(next);
        hs_proc.tag_stable = 0;
      } else if (owner.present) {
        hs_proc.tag_stable = !owner.unstable;
        if (owner.announced) {
          derive(next, inherited, 'x', 1);
          adopt(next);
        }
      }
    }
  }
  announced = 0;
  export_env();
  putenv(env_tag);
  putenv(env_pid);
}

void hs_identity_announced(void) {
  announced = 1;
  export_env();
}

void hs_identity_child(uint32_t n) {
  char next[HS_TAG_MAX + 1];
  derive(next, hs_proc.tag, 'f', n);
  adopt(next);
  announced = 0;
  export_env();
}
