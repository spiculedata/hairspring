/* callsite.c -- two call sites in their own functions, so their machine code can be read in isolation.
** `make disasm` prints it (docs/c-sdk.md quotes the result). Never linked into anything. */
#include "../../include/hairspring.h"

void site_always(long ok, long a, long b) { HS_ALWAYS(ok != 0, "disasm.always", "cond holds", a, b, 3, 4); }

void site_reach(void) { HS_REACHABLE("disasm.reach", "reached"); }
