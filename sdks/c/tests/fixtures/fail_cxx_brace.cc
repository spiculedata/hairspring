// A brace is dialect syntax in an x86 assembler template; it must be refused at compile time with g++.
#include "../../include/hairspring.h"

void f() { HS_REACHABLE("fail.brace", "a {b} message"); }
