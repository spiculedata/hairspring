/* Must NOT compile: a message of 513 bytes is over the protocol limit of 512. */
#include "../../include/hairspring.h"

#define X10 "xxxxxxxxxx"
#define X100 X10 X10 X10 X10 X10 X10 X10 X10 X10 X10

void f(void) { HS_REACHABLE("ok.id", X100 X100 X100 X100 X100 X10 "xxx"); }
