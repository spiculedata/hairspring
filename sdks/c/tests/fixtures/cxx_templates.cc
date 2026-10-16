// cxx_templates.cc -- call sites inside function templates, a class template member and a plain inline function.
// This file has NO call site in an ordinary (non-inline, non-template) function: g++ 13.3 rejects a file that mixes
// those with inline-function sites (see cxx_mixed.cc). The harness main is in cxx_templates_main.cc.
// Some compilers place the records of template call sites outside the catalog section; the SDK must then say so at
// report time (tests/check_cxx_templates.sh). Run with the events on stderr.
#include <cstdint>

#include "../../include/hairspring.h"

template <typename T>
void check_zero(T x) {
  HS_SOMETIMES(x == 0, "tmpl.zero", "zero seen", x);
}

template <typename T>
struct Gauge {
  void sample(T x) { HS_ALWAYS(x >= 0, "tmpl.member", "never negative", x); }
};

inline void check_plain(int x) { HS_ALWAYS(x > 0, "tmpl.inline", "positive", x); }

void run_template_sites() {
  check_plain(1);
  check_zero<int>(0);
  check_zero<long>(1);
  check_zero<std::uint8_t>(2);
  Gauge<int>().sample(4);
  Gauge<long>().sample(5);
}
