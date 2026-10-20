# C++ lost-update fixture (racy program and locked twin), built with the C SDK in sdks/c.
#
#   make                  build both programs with clang++ (or the compiler in CXX)
#   make test             build, then the native self-checks (see ../README.md)
#   make test CXX=g++     the same with g++; its call sites are all in ordinary functions, as docs/c-sdk.md advises
#   make clean
#
# Needs a C++17 compiler, POSIX threads and make. The SDK library is built with the C compiler of the same family
# (SDK_CC; clang for clang++, gcc for g++).
# Nothing here runs Hermit.

# clang++ unless the caller names a compiler (make's own default for CXX would be g++).
ifeq ($(origin CXX),default)
CXX := clang++
endif
# make defines CC itself, so the SDK's C compiler gets its own name.
SDK_CC ?= $(if $(findstring clang,$(CXX)),clang,$(if $(findstring g++,$(CXX)),gcc,cc))
OPT ?= -O2
BUILD ?= build/$(notdir $(CXX))$(subst -,,$(OPT))

SDK := ../../../sdks/c
SDK_BUILD := $(abspath $(BUILD))/sdk
LIB := $(SDK_BUILD)/libhairspring.a

CXXFLAGS += -std=gnu++17 $(OPT) -g -Wall -Wextra -Werror -pthread -I$(SDK)/include -I../common
LDFLAGS += -pthread

SRC := src/lost_update.cc
HDR := ../common/lang_at.h $(SDK)/include/hairspring.h
PROGRAMS := $(BUILD)/lost_update_racy $(BUILD)/lost_update_locked
RUNS ?= 50

.PHONY: all test clean FORCE

all: $(PROGRAMS)

# The SDK's own Makefile decides whether the library is up to date.
$(LIB): FORCE
	$(MAKE) -C $(SDK) lib CC="$(SDK_CC)" OPT="$(OPT)" BUILD="$(SDK_BUILD)"

FORCE:

$(BUILD)/lost_update_racy: $(SRC) $(HDR) $(LIB)
	mkdir -p $(BUILD)
	$(CXX) $(CXXFLAGS) -o $@ $(SRC) $(LIB) $(LDFLAGS)

$(BUILD)/lost_update_locked: $(SRC) $(HDR) $(LIB)
	mkdir -p $(BUILD)
	$(CXX) $(CXXFLAGS) -DLANG_AT_LOCKED -o $@ $(SRC) $(LIB) $(LDFLAGS)

# The negative control feeds the racy program in as the "locked twin": the check must reject it, or it checks nothing.
test: $(PROGRAMS)
	sh ../scripts/check-native.sh cpp $(BUILD)/lost_update_racy $(BUILD)/lost_update_locked $(RUNS)
	@if sh ../scripts/check-native.sh cpp $(BUILD)/lost_update_racy $(BUILD)/lost_update_racy $(RUNS) >/dev/null 2>&1; then \
	  echo "FAIL the self-check accepted the racy program as a locked twin"; exit 1; \
	else echo "ok   the self-check rejects a racy program standing in for the locked twin"; fi

clean:
	rm -rf build
