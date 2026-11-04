# Hairspring C SDK. Plain make; no dependencies beyond a C11 compiler (gcc or clang) and POSIX threads.
#
#   make test            unit, audit and process tests, the C++ checks (incl. audit-control), the no-syscall test (Linux)
#   make disasm         machine code of two call sites
#   make test CC=gcc     the same with another compiler (warnings are errors either way)
#   make tsan-signals   the process tests under TSan with its signal handlers off (runs the SIGSEGV report tests)
#   make asan-signals   the same under ASan + UBSan with ASan's signal handlers off
#   make conformance   fold the sample programs' output with the Rust parser (needs cargo)
#   make clean

CC ?= cc
OPT ?= -O2
BUILD ?= build/$(notdir $(CC))$(subst -,,$(OPT))

WARN := -Wall -Wextra -Werror
EXTRA ?=
CFLAGS += -std=gnu11 $(OPT) -g $(WARN) -pthread $(EXTRA)
LDFLAGS += -pthread

SDK_SRC := src/hairspring.c src/hs_audit.c src/hs_catalog.c src/hs_fmt.c src/hs_identity.c src/hs_lines.c src/hs_report.c
SDK_OBJ := $(SDK_SRC:src/%.c=$(BUILD)/%.o)
SDK_HDR := include/hairspring.h $(wildcard src/*.h)
LIB := $(BUILD)/libhairspring.a

SCENARIOS := basic merge fork threads crash exit_status eval big forkexec spawn closefd forklock execself dup2fd manyids conflict empty limits plain_exit segv abort threadsegv alive alarm evalfd sigign
SAMPLES := $(SCENARIOS:%=$(BUILD)/hs_sample_%)

.PHONY: all lib fixtures test unit audit proc tsan-signals asan-signals ubsan-fatal nosyscall nosyscall-pic cxx cxx-templates audit-control cxx-mixed cxx-staticfail cxx-utf8 staticfail gnusource disasm conformance clean

all: lib

lib: $(LIB)

fixtures: $(SAMPLES)

$(BUILD):
	mkdir -p $(BUILD)

$(BUILD)/%.o: src/%.c $(SDK_HDR) | $(BUILD)
	$(CC) $(CFLAGS) -c -o $@ $<

$(LIB): $(SDK_OBJ)
	rm -f $@
	ar rcs $@ $^

$(BUILD)/hs_sample_%: tests/fixtures/hs_sample.c $(LIB) $(SDK_HDR) | $(BUILD)
	$(CC) $(CFLAGS) -DHS_SCENARIO_$* -o $@ $< $(LIB) $(LDFLAGS)

$(BUILD)/test_unit: tests/test_unit.c tests/hs_test.h $(LIB) $(SDK_HDR) | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $< $(LIB) $(LDFLAGS)

$(BUILD)/test_audit: tests/test_audit.c tests/hs_test.h $(LIB) $(SDK_HDR) | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $< $(LIB) $(LDFLAGS)

# A call site outside the catalog section must be reported, not silently absent.
audit: $(BUILD)/test_audit
	HAIRSPRING_EVENTS=off $(BUILD)/test_audit

$(BUILD)/test_proc: tests/test_proc.c tests/hs_test.h | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $<

unit: $(BUILD)/test_unit
	HAIRSPRING_EVENTS=off $(BUILD)/test_unit

proc: $(BUILD)/test_proc $(SAMPLES)
	$(BUILD)/test_proc $(BUILD)

# The process tests under ThreadSanitizer with TSan's own fatal-signal handlers switched off (TSAN_OPTIONS handle_segv=0
# and friends, set by the test), so the SDK installs its handlers and the SIGSEGV report tests run instead of skipping.
# Not part of `make test`. Whether the runtime leaves the signals alone is the runtime's behaviour, not the SDK's.
TSAN_BUILD ?= build/tsan
ASAN_BUILD ?= build/asan-signals-$(notdir $(CC))
# -fno-sanitize-recover=undefined: a UBSan finding aborts the program, so it fails the test that hit it.
ASAN_FLAGS := -fsanitize=address,undefined -fno-sanitize-recover=undefined

tsan-signals:
	env HS_TEST_SANITIZER_SIGNALS=1 $(MAKE) proc BUILD=$(TSAN_BUILD) EXTRA=-fsanitize=thread

# The same under AddressSanitizer + UBSan with ASan's fatal-signal handlers off (ASAN_OPTIONS, set by the test).
asan-signals: ubsan-fatal
	env HS_TEST_SANITIZER_SIGNALS=1 $(MAKE) proc BUILD=$(ASAN_BUILD) EXTRA="$(ASAN_FLAGS)"

# Negative control: a seeded signed overflow must make the program exit non-zero under ASAN_FLAGS.
ubsan-fatal: | $(BUILD)
	$(CC) $(CFLAGS) $(ASAN_FLAGS) -o $(BUILD)/ubsan_probe tests/fixtures/ubsan_probe.c
	@if $(BUILD)/ubsan_probe >/dev/null 2>&1; then echo "FAIL a UBSan finding did not fail the run"; exit 1; \
	else echo "ok   a UBSan finding fails the run"; fi

# T1: no system call at a call site (seccomp; the test itself prints "skip" off Linux).
$(BUILD)/test_nosyscall: tests/test_nosyscall.c $(LIB) $(SDK_HDR) | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $< $(LIB) $(LDFLAGS)

nosyscall: $(BUILD)/test_nosyscall
	$(BUILD)/test_nosyscall

# Prints the machine code of two call sites (docs/c-sdk.md quotes it).
disasm: $(BUILD)/callsite.o
	@if command -v objdump >/dev/null 2>&1 && [ "$$(uname -s)" = Linux ]; then objdump -d --no-show-raw-insn $<; else otool -tvV $<; fi

$(BUILD)/callsite.o: tests/fixtures/callsite.c $(SDK_HDR) | $(BUILD)
	$(CC) $(CFLAGS) -c -o $@ $<

# The same test with everything built position-independent (a static archive linked into a PIE or a shared object).
nosyscall-pic:
	$(MAKE) nosyscall BUILD=$(BUILD)-pic EXTRA=-fPIC

# The macros must compile and report from C++ (needs a C++ compiler of the same family; skipped if absent).
CXXC := $(if $(findstring clang,$(CC)),clang++,$(if $(findstring gcc,$(CC)),g++,c++))

$(BUILD)/cxx_sample: tests/fixtures/cxx_sample.cc $(LIB) $(SDK_HDR) | $(BUILD)
	$(CXXC) -std=gnu++17 $(OPT) -g $(WARN) -pthread $(EXTRA) -o $@ $< $(LIB) $(LDFLAGS)

cxx: $(LIB)
	@if command -v $(CXXC) >/dev/null 2>&1; then $(MAKE) $(BUILD)/cxx_sample && \
	  out=$$($(BUILD)/cxx_sample 2>&1) && echo "$$out" | grep -q '"kind":"end"' && echo "ok   C++ sample reports" ; \
	else echo "skip: $(CXXC) not installed"; fi

# Call sites inside C++ templates must reach the catalog. clang places them in the section; g++ on ELF x86-64 and
# aarch64 gets there through inline assembly (see hairspring.h). Any other compiler or target may either do that or
# have the audit announce the loss (see docs/c-sdk.md), so it is only held to that.
CXX_IS_CLANG := $(filter-out 0,$(shell $(CXXC) --version 2>/dev/null | grep -c clang))
CXX_USES_ASM_SITES := $(and $(filter Linux,$(shell uname -s)),$(filter x86_64 aarch64,$(shell uname -m)),$(if $(CXX_IS_CLANG),,yes))
CXX_TEMPLATE_MODE := $(if $(or $(CXX_IS_CLANG),$(CXX_USES_ASM_SITES)),strict,either)

CXX_FIXTURE_SRC := tests/fixtures/cxx_templates.cc tests/fixtures/cxx_templates_main.cc

$(BUILD)/cxx_templates: $(CXX_FIXTURE_SRC) $(LIB) $(SDK_HDR) | $(BUILD)
	$(CXXC) -std=gnu++17 $(OPT) -g $(WARN) -pthread $(EXTRA) -o $@ $(CXX_FIXTURE_SRC) $(LIB) $(LDFLAGS)

cxx-templates: $(LIB)
	@if command -v $(CXXC) >/dev/null 2>&1; then $(MAKE) $(BUILD)/cxx_templates && \
	  env -u HAIRSPRING_EVENTS -u HAIRSPRING_EVENTS_FILE $(BUILD)/cxx_templates 2>$(BUILD)/cxx_templates.events && \
	  sh tests/check_cxx_templates.sh $(BUILD)/cxx_templates.events $(CXX_TEMPLATE_MODE) && \
	  sh tests/check_cxx_offline.sh $(BUILD)/cxx_templates $(CXX_TEMPLATE_MODE) ; \
	else echo "skip: $(CXXC) not installed"; fi

# Positive control for the audit: the same fixture built against a copy of the header whose records go to sections
# nobody reads. Every site is then missing from the catalog, whatever the compiler, and the report must announce all
# 7 evaluations (tmpl.zero 3, tmpl.member 2, tmpl.inline 1, tmpl.main 1).
CTL := $(BUILD)/ctl

audit-control: $(LIB) | $(BUILD)
	@if command -v $(CXXC) >/dev/null 2>&1; then \
	  rm -rf $(CTL) && mkdir -p $(CTL)/include $(CTL)/tests/fixtures && \
	  sed -e 's/hs_catalog_str_v1/hs_ctl_str_v1/; s/hs_catalog_v1/hs_ctl_v1/; s/__hs_catalog"/__hs_ctl"/; s/__hs_catstr"/__hs_ctlstr"/' include/hairspring.h > $(CTL)/include/hairspring.h && \
	  cp $(CXX_FIXTURE_SRC) $(CTL)/tests/fixtures/ && \
	  $(CXXC) -std=gnu++17 $(OPT) -g $(WARN) -pthread $(EXTRA) -o $(CTL)/cxx_lost $(CTL)/tests/fixtures/cxx_templates.cc $(CTL)/tests/fixtures/cxx_templates_main.cc $(LIB) $(LDFLAGS) && \
	  env -u HAIRSPRING_EVENTS -u HAIRSPRING_EVENTS_FILE $(CTL)/cxx_lost 2>$(CTL)/events && \
	  sh tests/check_cxx_templates.sh $(CTL)/events either | tee $(CTL)/verdict && \
	  grep -q 'announced: 7 evaluation(s)' $(CTL)/verdict && echo "ok   audit announces a catalog that lost every site"; \
	else echo "skip: $(CXXC) not installed"; fi

# The mixed-file check: one inline-function site and one ordinary-function site in a file. clang and g++ with the
# assembly records (see hairspring.h) must compile it and catalogue both; g++ 13.3 with section attributes rejected it
# ("section type conflict"), which is what any other compiler or target is expected to still do.
CXX_MIXED_MODE := $(if $(or $(CXX_IS_CLANG),$(CXX_USES_ASM_SITES)),catalogued,other)

cxx-mixed: | $(BUILD)
	@if command -v $(CXXC) >/dev/null 2>&1; then $(MAKE) $(LIB) && \
	  if $(CXXC) -std=gnu++17 $(OPT) -g -pthread $(EXTRA) -o $(BUILD)/cxx_mixed tests/fixtures/cxx_mixed.cc $(LIB) $(LDFLAGS) 2>$(BUILD)/cxx_mixed.log; then \
	    env -u HAIRSPRING_EVENTS -u HAIRSPRING_EVENTS_FILE $(BUILD)/cxx_mixed 2>$(BUILD)/cxx_mixed.events; \
	  else : >$(BUILD)/cxx_mixed.events; fi; \
	  sh tests/check_cxx_mixed.sh $(BUILD)/cxx_mixed.log $(BUILD)/cxx_mixed.events $(CXX_MIXED_MODE); \
	else echo "skip: $(CXXC) not installed"; fi

# With the assembly records an id, a message or a file name with a character that is special in the assembler template
# must be rejected at compile time, not mis-assembled. Only meaningful where the assembly records are used.
cxx-staticfail: | $(BUILD)
	@if [ -n "$(CXX_USES_ASM_SITES)" ] && command -v $(CXXC) >/dev/null 2>&1; then \
	  for f in tests/fixtures/fail_cxx_*.cc; do \
	    if $(CXXC) -std=gnu++17 $(OPT) -fsigned-char -fsyntax-only $$f 2>$(BUILD)/fail.log; then echo "FAIL $$f compiled"; exit 1; \
	    elif grep -q 'must not contain' $(BUILD)/fail.log; then echo "ok   $$f is rejected"; \
	    else echo "FAIL $$f was rejected for another reason"; cat $(BUILD)/fail.log; exit 1; fi; done; \
	else echo "skip: the assembly records are not used with $(CXXC) here"; fi

# UTF-8 text (bytes of 0x80 and above) is accepted in an id, a message and a file name whether `char` is signed or
# unsigned; a directory with a non-ASCII name puts such bytes in __FILE__ as well.
cxx-utf8: | $(BUILD)
	@if command -v $(CXXC) >/dev/null 2>&1; then \
	  d="$(BUILD)/$$(printf 'caf\303\251')" && mkdir -p "$$d" && cp tests/fixtures/ok_cxx_utf8.cc "$$d/" && \
	  for sign in -fsigned-char -funsigned-char; do \
	    for src in tests/fixtures/ok_cxx_utf8.cc "$$d/ok_cxx_utf8.cc"; do \
	      $(CXXC) -std=gnu++17 $(OPT) $$sign $(WARN) -Iinclude -c -o $(BUILD)/utf8.o "$$src" || { echo "FAIL $$src with $$sign"; exit 1; }; \
	    done; done; \
	  echo "ok   UTF-8 text is accepted with signed and unsigned char"; \
	else echo "skip: $(CXXC) not installed"; fi

# These must be rejected at compile time.
staticfail:
	@for f in tests/fixtures/fail_*.c; do \
	  if $(CC) $(CFLAGS) -fsyntax-only $$f 2>/dev/null; then echo "FAIL $$f compiled"; exit 1; \
	  else echo "ok   $$f is rejected"; fi; done

# A build that already defines _GNU_SOURCE must not trip -Werror on a redefinition.
gnusource: | $(BUILD)
	$(CC) $(CFLAGS) -D_GNU_SOURCE -c -o $(BUILD)/gnusource.o src/hairspring.c && echo "ok   builds with -D_GNU_SOURCE"

test: unit audit proc nosyscall nosyscall-pic staticfail cxx cxx-templates audit-control cxx-mixed cxx-staticfail cxx-utf8 gnusource

conformance: $(SAMPLES)
	cd ../.. && HS_C_SDK_BUILD=$(abspath $(BUILD)) cargo test -p hairspring-props --test c_sdk

clean:
	rm -rf build
