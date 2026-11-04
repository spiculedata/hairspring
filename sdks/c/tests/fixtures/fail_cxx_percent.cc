// With g++ the record is written by inline assembly, whose template treats `%` specially; such a message must be
// refused at compile time (see HS__PLAIN in hairspring.h), not mis-assembled.
#include "../../include/hairspring.h"

void f() { HS_REACHABLE("fail.percent", "100% done"); }
