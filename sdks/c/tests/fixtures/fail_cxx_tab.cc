// A tab is a control character and must be refused with g++.
#include "../../include/hairspring.h"

void f() { HS_REACHABLE("fail.tab", "a\tmessage"); }
