# condlock: the glibc demo workload (lock + barrier over pthread condvars) and its detector self-tests.
# Plain make, C11, POSIX threads, the repo's C SDK (sdks/c, pinned in PINS). Native builds are informational.
#
#   make unit            unit tests (rule, config, report) - no threads
#   make condlock        the counted workload (SDK build)
#   make selftest-bins   the planted builds (D-pos, D-stall) and the violation-only / no-watchdog variants
#   make test            unit + workload behaviour + classifier tests
#   make clean
#
# Planted builds are separate binaries under build/planted; the counted binary contains no planted code.

CC ?= cc
OPT ?= -O2
HERE := $(abspath .)
BUILD ?= $(HERE)/build
SDK := $(abspath ../../sdks/c)
SDK_BUILD := $(BUILD)/sdk
SDK_LIB := $(SDK_BUILD)/libhairspring.a

WARN := -Wall -Wextra -Werror
CFLAGS += -std=gnu11 $(OPT) -g $(WARN) -pthread -Isrc -I$(SDK)/include -I$(SDK)/tests
LDFLAGS += -pthread

# Everything except main.c: the unit tests link the pure modules only, the binaries link all of it.
PURE_SRC := src/cl_rule.c src/cl_config.c src/cl_report.c src/cl_condraw.c
CORE_SRC := $(PURE_SRC) src/cl_time.c src/cl_check.c src/cl_lock.c src/cl_worker.c src/cl_watchdog.c
HDR := $(wildcard src/*.h)

.PHONY: all unit condlock selftest-bins tprobe test clean sdk

all: condlock tprobe

sdk: $(SDK_LIB)

$(SDK_LIB):
	mkdir -p $(SDK_BUILD)
	$(MAKE) -C $(SDK) lib BUILD=$(SDK_BUILD) CC="$(CC)" OPT="$(OPT)"

$(BUILD)/unit:
	mkdir -p $@

$(BUILD)/unit/%: tests/%.c $(PURE_SRC) $(HDR) | $(BUILD)/unit
	$(CC) $(CFLAGS) -o $@ $< $(PURE_SRC) $(LDFLAGS)

# cl_condraw.c against stand-in glibc headers (tests/stand-in), one per arm layout.
$(BUILD)/unit/test_condraw_u: tests/test_condraw.c src/cl_condraw.c $(HDR) tests/stand-in/arm-u/pthread.h | $(BUILD)/unit
	$(CC) $(CFLAGS) -D__GLIBC__=2 -DCONDLOCK_ARM_U -Itests/stand-in/arm-u -o $@ tests/test_condraw.c src/cl_condraw.c $(LDFLAGS)

$(BUILD)/unit/test_condraw_c: tests/test_condraw.c src/cl_condraw.c $(HDR) tests/stand-in/arm-c/pthread.h | $(BUILD)/unit
	$(CC) $(CFLAGS) -D__GLIBC__=2 -DCONDLOCK_ARM_C -Itests/stand-in/arm-c -o $@ tests/test_condraw.c src/cl_condraw.c $(LDFLAGS)

unit: $(BUILD)/unit/test_rule $(BUILD)/unit/test_config $(BUILD)/unit/test_report $(BUILD)/unit/test_condraw_u $(BUILD)/unit/test_condraw_c
	$(BUILD)/unit/test_rule
	$(BUILD)/unit/test_config
	$(BUILD)/unit/test_report
	$(BUILD)/unit/test_condraw_u
	$(BUILD)/unit/test_condraw_c

$(BUILD)/condlock: src/main.c $(CORE_SRC) $(HDR) $(SDK_LIB)
	$(CC) $(CFLAGS) -o $@ src/main.c $(CORE_SRC) $(SDK_LIB) $(LDFLAGS)

condlock: $(BUILD)/condlock

$(BUILD)/tprobe: src/tprobe.c
	$(CC) $(CFLAGS) -o $@ $< $(LDFLAGS)

tprobe: $(BUILD)/tprobe

# Variants. Each is its own file with its own flags.
PLANT_DIR := $(BUILD)/planted
DROP_KS := 1 2 5 20 50
LEAK_KS := 1 2 5 20 50
BARRIER_KS := 1 2 5 20 50
HOLDG_KS := 1 2 5 20 50
PLANTED := $(DROP_KS:%=$(PLANT_DIR)/condlock-drop-k%) $(LEAK_KS:%=$(PLANT_DIR)/condlock-leak-k%) \
           $(BARRIER_KS:%=$(PLANT_DIR)/condlock-barrier-k%) $(HOLDG_KS:%=$(PLANT_DIR)/condlock-holdg-k%)

$(PLANT_DIR):
	mkdir -p $@

$(PLANT_DIR)/condlock-drop-k%: src/main.c $(CORE_SRC) $(HDR) $(SDK_LIB) | $(PLANT_DIR)
	$(CC) $(CFLAGS) -DCONDLOCK_DROP_SIGNAL=$* -o $@ src/main.c $(CORE_SRC) $(SDK_LIB) $(LDFLAGS)

$(PLANT_DIR)/condlock-leak-k%: src/main.c $(CORE_SRC) $(HDR) $(SDK_LIB) | $(PLANT_DIR)
	$(CC) $(CFLAGS) -DCONDLOCK_PLANT_LEAK=$* -o $@ src/main.c $(CORE_SRC) $(SDK_LIB) $(LDFLAGS)

$(PLANT_DIR)/condlock-barrier-k%: src/main.c $(CORE_SRC) $(HDR) $(SDK_LIB) | $(PLANT_DIR)
	$(CC) $(CFLAGS) -DCONDLOCK_PLANT_BARRIER=$* -o $@ src/main.c $(CORE_SRC) $(SDK_LIB) $(LDFLAGS)

$(PLANT_DIR)/condlock-holdg-k%: src/main.c $(CORE_SRC) $(HDR) $(SDK_LIB) | $(PLANT_DIR)
	$(CC) $(CFLAGS) -DCONDLOCK_PLANT_HOLDG=$* -o $@ src/main.c $(CORE_SRC) $(SDK_LIB) $(LDFLAGS)

# Violation-only (no SDK), with a drop plant; and the no-watchdog variant (healthy only, informational).
$(PLANT_DIR)/condlock-novio-drop-k1: src/main.c $(CORE_SRC) $(HDR) | $(PLANT_DIR)
	$(CC) $(CFLAGS) -DCONDLOCK_NO_SDK -DCONDLOCK_DROP_SIGNAL=1 -o $@ src/main.c $(CORE_SRC) $(LDFLAGS)

$(BUILD)/condlock-novio: src/main.c $(CORE_SRC) $(HDR)
	$(CC) $(CFLAGS) -DCONDLOCK_NO_SDK -o $@ src/main.c $(CORE_SRC) $(LDFLAGS)

$(BUILD)/condlock-nowd: src/main.c $(CORE_SRC) $(HDR) $(SDK_LIB)
	$(CC) $(CFLAGS) -DCONDLOCK_NOWD -o $@ src/main.c $(CORE_SRC) $(SDK_LIB) $(LDFLAGS)

selftest-bins: $(PLANTED) $(PLANT_DIR)/condlock-novio-drop-k1 $(BUILD)/condlock-novio $(BUILD)/condlock-nowd

test: unit condlock tprobe selftest-bins
	sh tests/test_tprobe.sh $(BUILD)
	sh tests/test_workload.sh $(BUILD)
	sh tests/test_classify.sh $(BUILD)
	sh tests/test_results_guard.sh $(BUILD)

clean:
	rm -rf build
