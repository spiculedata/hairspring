// cxx_mixed.cc -- one call site in an inline function and one in an ordinary function, in the SAME file.
// Observed with g++ 13.3 on Linux x86-64: this does not compile ("section type conflict" for hs__site and hs__offline).
// clang compiles it and catalogues both sites. `make cxx-mixed` asserts exactly that, per compiler.
#include "../../include/hairspring.h"

inline void check_inline(int x) { HS_ALWAYS(x > 0, "mixed.inline", "positive", x); }

int main() {
  check_inline(1);
  HS_REACHABLE("mixed.ordinary", "reached");
  return 0;
}
