/* cl_config.h -- command line: -n N -r R -tick-ms T -q Q. Nothing else is read (no environment variable). */
#ifndef CL_CONFIG_H
#define CL_CONFIG_H

#include <stdint.h>

#define CL_DEFAULT_WORKERS 8u
#define CL_DEFAULT_ROUNDS 500u
#define CL_DEFAULT_TICK_MS 100u
#define CL_DEFAULT_QUORUM 10u

#define CL_MAX_ROUNDS 1000000u
#define CL_MAX_TICK_MS 10000u
#define CL_MAX_QUORUM 1000u

#define CL_EXIT_USAGE 64

struct cl_config {
  unsigned workers;
  unsigned rounds;
  unsigned tick_ms;
  unsigned quorum;
};

/* Fills *out with the defaults, then applies argv[1..]. Returns 0 on success; on any error returns -1 and writes a
 * one-line reason to err (at most err_len bytes). Unknown flags, repeated values, missing values and out-of-range
 * values are all errors. */
int cl_config_parse(struct cl_config *out, int argc, char **argv, char *err, unsigned err_len);

#endif
