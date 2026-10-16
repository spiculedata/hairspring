// cxx_sample.cc -- the macros must work from C++ (values of mixed types, no value, every kind).
#include <cstdint>

#include "../../include/hairspring.h"

int main() {
  std::uint64_t big = 5;
  int small = -3;
  for (int i = 0; i < 4; i++) HS_ALWAYS(i >= 0, "cxx.always", "holds", big, small, i);
  HS_ALWAYS_OR_UNREACHABLE(true, "cxx.aou", "holds");
  HS_SOMETIMES(small < 0, "cxx.sometimes", "holds", small);
  HS_REACHABLE("cxx.reach", "reached", 1, 2, 3, 4);
  HS_WORK(2);
  return 0;
}
