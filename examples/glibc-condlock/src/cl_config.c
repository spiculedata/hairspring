#include "cl_config.h"

#include <stdio.h>
#include <string.h>

/* Strict decimal: digits only, no sign, no space, no overflow past `max`. */
static int parse_uint(const char *s, unsigned min, unsigned max, unsigned *out) {
  if (s == NULL || *s == '\0') return -1;
  unsigned long v = 0;
  for (; *s != '\0'; s++) {
    if (*s < '0' || *s > '9') return -1;
    v = v * 10ul + (unsigned long)(*s - '0');
    if (v > max) return -1;
  }
  if (v < min) return -1;
  *out = (unsigned)v;
  return 0;
}

static void fail(char *err, unsigned err_len, const char *what, const char *arg) {
  if (err != NULL && err_len > 0) snprintf(err, err_len, "%s: %s", what, arg != NULL ? arg : "(none)");
}

struct flag_spec {
  const char *name;
  unsigned min, max;
  unsigned *dest;
};

int cl_config_parse(struct cl_config *out, int argc, char **argv, char *err, unsigned err_len) {
  out->workers = CL_DEFAULT_WORKERS;
  out->rounds = CL_DEFAULT_ROUNDS;
  out->tick_ms = CL_DEFAULT_TICK_MS;
  out->quorum = CL_DEFAULT_QUORUM;

  const struct flag_spec specs[] = {
      {"-n", 1, 16, &out->workers},
      {"-r", 1, CL_MAX_ROUNDS, &out->rounds},
      {"-tick-ms", 1, CL_MAX_TICK_MS, &out->tick_ms},
      {"-q", 1, CL_MAX_QUORUM, &out->quorum},
  };
  const unsigned nspecs = (unsigned)(sizeof specs / sizeof specs[0]);
  int seen[sizeof specs / sizeof specs[0]] = {0};

  for (int i = 1; i < argc; i += 2) {
    unsigned k = 0;
    while (k < nspecs && strcmp(argv[i], specs[k].name) != 0) k++;
    if (k == nspecs) {
      fail(err, err_len, "unknown flag", argv[i]);
      return -1;
    }
    if (seen[k]) {
      fail(err, err_len, "repeated flag", argv[i]);
      return -1;
    }
    seen[k] = 1;
    if (i + 1 >= argc) {
      fail(err, err_len, "missing value for", argv[i]);
      return -1;
    }
    if (parse_uint(argv[i + 1], specs[k].min, specs[k].max, specs[k].dest) != 0) {
      fail(err, err_len, "bad value for flag", argv[i]);
      return -1;
    }
  }
  return 0;
}
