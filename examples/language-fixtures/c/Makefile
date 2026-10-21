# C lost-update fixture (racy program and locked twin), built with the C SDK in sdks/c.
#
#   make              build both programs
#   make test         build, then the native self-checks (see ../README.md)
#   make test CC=gcc  the same with another compiler (clang and gcc both work)
#   make clean
#
# Needs a C11 compiler, POSIX threads and make. Nothing here runs Hermit.

CC ?= cc
OPT ?= -O2
BUILD ?= build/$(notdir $(CC))$(subst -,,$(OPT))

SDK := ../../../sdks/c
SDK_BUILD := $(abspath $(BUILD))/sdk
LIB := $(SDK_BUILD)/libhairspring.a

CFLAGS += -std=gnu11 $(OPT) -g -Wall -Wextra -Werror -pthread -I$(SDK)/include -I../common
LDFLAGS += -pthread

SRC := src/lost_update.c
HDR := ../common/lang_at.h $(SDK)/include/hairspring.h
PROGRAMS := $(BUILD)/lost_update_racy $(BUILD)/lost_update_locked
RUNS ?= 50

.PHONY: all test clean FORCE

all: $(PROGRAMS)

# The SDK's own Makefile decides whether the library is up to date.
$(LIB): FORCE
	$(MAKE) -C $(SDK) lib CC="$(CC)" OPT="$(OPT)" BUILD="$(SDK_BUILD)"

FORCE:

$(BUILD)/lost_update_racy: $(SRC) $(HDR) $(LIB)
	mkdir -p $(BUILD)
	$(CC) $(CFLAGS) -o $@ $(SRC) $(LIB) $(LDFLAGS)

$(BUILD)/lost_update_locked: $(SRC) $(HDR) $(LIB)
	mkdir -p $(BUILD)
	$(CC) $(CFLAGS) -DLANG_AT_LOCKED -o $@ $(SRC) $(LIB) $(LDFLAGS)

# The negative control feeds the racy program in as the "locked twin": the check must reject it, or it checks nothing.
test: $(PROGRAMS)
	sh ../scripts/run-checks.sh c $(BUILD)/lost_update_racy $(BUILD)/lost_update_locked $(RUNS)

clean:
	rm -rf build
