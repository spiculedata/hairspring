// DEL is a control character and must be refused with g++.
#include "../../include/hairspring.h"

void f() { HS_REACHABLE("fail.del", "a\x7f message"); }
