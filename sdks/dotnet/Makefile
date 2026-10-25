# Hairspring guest SDK for .NET: C#, net10.0, base class library only.
#
#   make dll        build/artifacts/bin/Hairspring/release/Hairspring.dll (reproducible; see src/Hairspring.csproj)
#   make test       the self-test of the classes, then the shell tests (the sample programs as real processes)
#   make selftest   only the self-test
#   make samples    build the sample programs
#   make process    only the shell tests
#   make clean
#
# Needs the .NET 10 SDK (DOTNET=/path/to/dotnet to choose; `dotnet --version` shows which one this is). Everything is built
# below $(ARTIFACTS), so the source tree gets no bin or obj directories from this Makefile. Nothing here runs Hermit.

DOTNET ?= dotnet
BUILD ?= build
ARTIFACTS ?= $(BUILD)/artifacts
BUILD_FLAGS := -c Release --artifacts-path $(ARTIFACTS) --nologo -v q --disable-build-servers

.PHONY: dll test selftest samples process clean

dll:
	$(DOTNET) build src/Hairspring.csproj $(BUILD_FLAGS)

samples:
	$(DOTNET) build samples/Sample/Sample.csproj $(BUILD_FLAGS)

test: selftest process

selftest:
	$(DOTNET) run --project tests/SelfTest/SelfTest.csproj $(BUILD_FLAGS)

process: samples
	DOTNET="$(DOTNET)" sh test/test-process.sh $(ARTIFACTS)/bin/Sample/release/Sample.dll

clean:
	rm -rf $(BUILD)
