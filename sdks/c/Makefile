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
CFLAGS += -std=gnu11 $(OPT) -g $(WARN) -pthread
LDFLAGS += -pthread

SDK_SRC := src/hairspring.c src/hs_catalog.c src/hs_fmt.c src/hs_lines.c src/hs_report.c
SDK_OBJ := $(SDK_SRC:src/%.c=$(BUILD)/%.o)
SDK_HDR := include/hairspring.h $(wildcard src/*.h)
LIB := $(BUILD)/libhairspring.a

SCENARIOS := basic merge fork threads crash exit_status eval big small
SAMPLES := $(SCENARIOS:%=$(BUILD)/hs_sample_%)

.PHONY: all lib fixtures test unit proc nosyscall disasm conformance clean

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

test: unit proc nosyscall

conformance: $(SAMPLES)
	cd ../.. && HS_C_SDK_BUILD=$(abspath $(BUILD)) cargo test -p hairspring-props --test c_sdk

clean:
	rm -rf build
