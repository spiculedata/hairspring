# Hairspring C SDK. Plain make; no dependencies beyond a C11 compiler (gcc or clang) and POSIX threads.
#
#   make test            unit tests + process tests + the no-syscall test (Linux)
#   make disasm         machine code of two call sites
#   make test CC=gcc     the same with another compiler (warnings are errors either way)
#   make conformance     fold the sample programs' output with the Rust parser (needs cargo)
#   make clean

CC ?= cc
OPT ?= -O2
BUILD ?= build/$(notdir $(CC))$(subst -,,$(OPT))

WARN := -Wall -Wextra -Werror
EXTRA ?=
CFLAGS += -std=gnu11 $(OPT) -g $(WARN) -pthread $(EXTRA)
LDFLAGS += -pthread

SDK_SRC := src/hairspring.c src/hs_catalog.c src/hs_fmt.c src/hs_identity.c src/hs_lines.c src/hs_report.c
SDK_OBJ := $(SDK_SRC:src/%.c=$(BUILD)/%.o)
SDK_HDR := include/hairspring.h $(wildcard src/*.h)
LIB := $(BUILD)/libhairspring.a

SCENARIOS := basic merge fork threads crash exit_status eval big forkexec spawn closefd forklock execself dup2fd manyids conflict empty limits plain_exit segv abort threadsegv alive alarm evalfd sigign
SAMPLES := $(SCENARIOS:%=$(BUILD)/hs_sample_%)

.PHONY: all lib fixtures test unit proc nosyscall nosyscall-pic cxx staticfail disasm conformance clean

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

$(BUILD)/test_proc: tests/test_proc.c tests/hs_test.h | $(BUILD)
	$(CC) $(CFLAGS) -o $@ $<

unit: $(BUILD)/test_unit
	HAIRSPRING_EVENTS=off $(BUILD)/test_unit

proc: $(BUILD)/test_proc $(SAMPLES)
	$(BUILD)/test_proc $(BUILD)

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
	$(CXXC) -std=gnu++17 $(OPT) -g $(WARN) -pthread -o $@ $< $(LIB) $(LDFLAGS)

cxx:
	@if command -v $(CXXC) >/dev/null 2>&1; then $(MAKE) $(BUILD)/cxx_sample && \
	  out=$$($(BUILD)/cxx_sample 2>&1) && echo "$$out" | grep -q '"kind":"end"' && echo "ok   C++ sample reports" ; \
	else echo "skip: $(CXXC) not installed"; fi

# These must be rejected at compile time.
staticfail:
	@for f in tests/fixtures/fail_*.c; do \
	  if $(CC) $(CFLAGS) -fsyntax-only $$f 2>/dev/null; then echo "FAIL $$f compiled"; exit 1; \
	  else echo "ok   $$f is rejected"; fi; done

test: unit proc nosyscall nosyscall-pic staticfail cxx

conformance: $(SAMPLES)
	cd ../.. && HS_C_SDK_BUILD=$(abspath $(BUILD)) cargo test -p hairspring-props --test c_sdk

clean:
	rm -rf build
