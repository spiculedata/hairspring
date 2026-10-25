# .NET probe (LangAtProbe.dll): a BCL-only program used to check how .NET threads are scheduled. See ../README.md.
#
#   make test     the unit checks, then the build, then the native tests of the built program
#   make unit     only the unit checks (selftest/)
#   make build    publish the framework-dependent program into build/publish (LangAtProbe.dll, .runtimeconfig.json, .deps.json)
#   make native   build, then run test_probe.sh against the result
#   make clean
#
# The lost-update fixture (LangAt.dll, with the .NET SDK in sdks/dotnet beside it as Hairspring.dll; sources in at/) has its own
# targets:
#
#   make at-build         publish the fixture into build/at (LangAt.dll, LangAt.runtimeconfig.json, LangAt.deps.json, Hairspring.dll)
#   make at-test          the fixture's self-test, source check, and native self-checks (the default environment and each profile
#                         arm of profiles.manifest)
#   make at-selftest      only the self-test of the fixture's classes
#   make at-sources       only the source check (no seed, /proc, environment, CPU count or clock in the fixture's sources)
#   make at-native        only the native self-check and the negative control, in the default environment
#   make at-profile-native  the same self-check once under each profile arm
#
# Needs the .NET 10 SDK (DOTNET=/path/to/dotnet to choose). The output is framework-dependent: it runs with
# `dotnet LangAtProbe.dll` (or `dotnet LangAt.dll <mode>`) on any .NET 10 runtime. Nothing here runs Hermit.

DOTNET ?= dotnet
RUNS ?= 5
PROJECT := src/LangAtProbe.csproj
OUT ?= build/publish
# The same flags the build record uses; --no-restore needs the restore step first.
PUBLISH_FLAGS := -c Release -p:Deterministic=true -p:ContinuousIntegrationBuild=true --no-restore --nologo -v q

.PHONY: test unit build native clean at-build at-test at-selftest at-sources at-native at-profile-native

test: unit build native

unit:
	$(DOTNET) run --project selftest/SelfTest.csproj --nologo -v q

build:
	$(DOTNET) restore $(PROJECT) --nologo -v q
	$(DOTNET) publish $(PROJECT) $(PUBLISH_FLAGS) -o $(OUT)

native: build
	sh test_probe.sh "$$(command -v $(DOTNET))" $(OUT)/LangAtProbe.dll $(RUNS)

# ---- the lost-update fixture: LangAt.dll with Hairspring.dll --------------------------------------------------------------

RUNS_AT ?= 50
AT_PROJECT := at/LangAt.csproj
AT_OUT ?= build/at

at-build:
	$(DOTNET) restore $(AT_PROJECT) --nologo -v q
	$(DOTNET) publish $(AT_PROJECT) $(PUBLISH_FLAGS) -o $(AT_OUT)

at-test: at-selftest at-sources at-native at-profile-native

# Run with HAIRSPRING_EVENTS=off so the SDK writes no report for the evaluations the tests make.
at-selftest:
	HAIRSPRING_EVENTS=off $(DOTNET) run --project at/test/AtSelfTest.csproj --nologo -v q

# No read of the seed, /proc, the environment, the CPU count or the clock in the fixture's own sources.
at-sources:
	sh ../scripts/check-sources-dotnet.sh at/src

# The program as the self-checks run it: a command that takes the mode as its first argument.
$(AT_OUT)/at-dotnet: at-build
	printf '#!/bin/sh\nexec "%s" "%s" "$$@"\n' "$$(command -v $(DOTNET))" "$(abspath $(AT_OUT))/LangAt.dll" >$@
	chmod +x $@

# The contract, natively, 50 runs of buggy and fixed (buggy must fire at least once, fixed never), wide as a contract
# check, and a negative control: the racy program standing in for fixed must be rejected.
at-native: $(AT_OUT)/at-dotnet
	sh ../scripts/run-checks.sh dotnet $(AT_OUT)/at-dotnet $(RUNS_AT)

# The same self-checks under each profile arm's environment variables (profiles.manifest).
at-profile-native: at-build
	sh ../scripts/run-dotnet-profile-checks.sh profiles.manifest $(AT_OUT)/LangAt.dll "$$(command -v $(DOTNET))" $(RUNS_AT)

clean:
	rm -rf build src/bin src/obj selftest/bin selftest/obj at/bin at/obj at/test/bin at/test/obj
