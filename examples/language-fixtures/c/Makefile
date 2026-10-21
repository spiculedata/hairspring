# C lost-update fixture: one program, at-c, with the modes buggy, fixed and wide, built with the C SDK in sdks/c.
#
#   make              build at-c
#   make test         build, then the source check and the native self-checks (see ../README.md)
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
PROGRAM := $(BUILD)/at-c
RUNS ?= 50

.PHONY: all test clean FORCE

all: $(PROGRAM)

# The SDK's own Makefile decides whether the library is up to date.
$(LIB): FORCE
	$(MAKE) -C $(SDK) lib CC="$(CC)" OPT="$(OPT)" BUILD="$(SDK_BUILD)"

FORCE:

$(PROGRAM): $(SRC) $(HDR) $(LIB)
	mkdir -p $(BUILD)
	$(CC) $(CFLAGS) -o $@ $(SRC) $(LIB) $(LDFLAGS)

# The negative control runs the buggy mode in place of fixed: the check must reject it, or it checks nothing.
test: $(PROGRAM)
	sh ../scripts/check-sources.sh $(SRC) ../common/lang_at.h
	sh ../scripts/run-checks.sh c $(PROGRAM) $(RUNS)

clean:
	rm -rf build
