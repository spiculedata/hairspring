#include "cl_check.h"

#include <stdio.h>
#include <string.h>

#include "cl_sdk.h"

void cl_ok(int rc, const char *what) {
  HS_ALWAYS(rc == 0, "condlock.harness.pthread_ok", "every pthread call in the workload returns 0", rc);
  if (rc != 0) {
    fprintf(stderr, "condlock: %s failed: %s\n", what, strerror(rc));
    hs_exit(CL_EXIT_HARNESS);
  }
}
