/* Must NOT compile: at most four values can be captured. */
#include "../../include/hairspring.h"

void f(long a) { HS_ALWAYS(a > 0, "ok.id", "five values", 1, 2, 3, 4, 5); }
