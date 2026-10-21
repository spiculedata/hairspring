# C++ lost-update fixture: one program per compiler, with the modes buggy, fixed and wide, built with the C SDK in
# sdks/c. The artifact is at-cxx-clang (clang++, the default) or at-cxx-gxx (g++).
#
#   make                  build the artifact of the compiler in CXX
#   make test             build, then the source check and the native self-checks (see ../README.md)
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
# The artifact is named for the compiler family: at-cxx-clang or at-cxx-gxx.
FAMILY := $(if $(findstring clang,$(CXX)),clang,$(if $(findstring g++,$(CXX)),gxx,other))
OPT ?= -O2
BUILD ?= build/$(notdir $(CXX))$(subst -,,$(OPT))

SDK := ../../../sdks/c
SDK_BUILD := $(abspath $(BUILD))/sdk
LIB := $(SDK_BUILD)/libhairspring.a

CXXFLAGS += -std=gnu++17 $(OPT) -g -Wall -Wextra -Werror -pthread -I$(SDK)/include -I../common
LDFLAGS += -pthread

SRC := src/lost_update.cc
HDR := ../common/lang_at.h $(SDK)/include/hairspring.h
PROGRAM := $(BUILD)/at-cxx-$(FAMILY)
RUNS ?= 50

.PHONY: all test clean FORCE

all: $(PROGRAM)

# The SDK's own Makefile decides whether the library is up to date.
$(LIB): FORCE
	$(MAKE) -C $(SDK) lib CC="$(SDK_CC)" OPT="$(OPT)" BUILD="$(SDK_BUILD)"

FORCE:

$(PROGRAM): $(SRC) $(HDR) $(LIB)
	mkdir -p $(BUILD)
	$(CXX) $(CXXFLAGS) -o $@ $(SRC) $(LIB) $(LDFLAGS)

# The negative control runs the buggy mode in place of fixed: the check must reject it, or it checks nothing.
test: $(PROGRAM)
	sh ../scripts/check-sources.sh $(SRC) ../common/lang_at.h
	sh ../scripts/run-checks.sh cxx $(PROGRAM) $(RUNS)

clean:
	rm -rf build
