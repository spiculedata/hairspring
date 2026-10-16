// cxx_templates_main.cc -- the harness for cxx_templates.cc. Its one call site is in an ordinary function and is the
// only site in this file, so no file mixes inline-function and ordinary-function sites.
#include "../../include/hairspring.h"

void run_template_sites();

int main() {
  run_template_sites();
  HS_REACHABLE("tmpl.main", "reached");
  return 0;
}
