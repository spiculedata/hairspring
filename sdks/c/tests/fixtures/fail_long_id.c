/* Must NOT compile: an id of 129 bytes is over the protocol limit of 128. */
#include "../../include/hairspring.h"

#define X10 "xxxxxxxxxx"
#define X100 X10 X10 X10 X10 X10 X10 X10 X10 X10 X10

void f(void) { HS_REACHABLE(X100 X10 X10 "xxxxxxxxx", "too long an id"); }
