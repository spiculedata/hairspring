/* ubsan_probe.c -- a deliberate signed overflow. `make ubsan-fatal` builds it with the flags of the ASan+UBSan builds
** and requires it to exit non-zero, which proves a UBSan finding fails the run (and not only prints a line). */
#include <limits.h>
#include <stdio.h>

int main(void) {
  volatile int big = INT_MAX; /* volatile: the compiler cannot fold the overflow away */
  int wrapped = big + 1;
  printf("%d\n", wrapped);
  return 0;
}
