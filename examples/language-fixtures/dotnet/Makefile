# .NET probe (LangAtProbe.dll): a BCL-only program used to check how .NET threads are scheduled. See ../README.md.
#
#   make test     the unit checks, then the build, then the native tests of the built program
#   make unit     only the unit checks (selftest/)
#   make build    publish the framework-dependent program into build/publish (LangAtProbe.dll, .runtimeconfig.json, .deps.json)
#   make native   build, then run test_probe.sh against the result
#   make clean
#
# Needs the .NET 10 SDK (DOTNET=/path/to/dotnet to choose). The output is framework-dependent: it runs with
# `dotnet LangAtProbe.dll` on any .NET 10 runtime. Nothing here runs Hermit.

DOTNET ?= dotnet
RUNS ?= 5
PROJECT := src/LangAtProbe.csproj
OUT ?= build/publish
# The same flags the build record uses; --no-restore needs the restore step first.
PUBLISH_FLAGS := -c Release -p:Deterministic=true -p:ContinuousIntegrationBuild=true --no-restore --nologo -v q

.PHONY: test unit build native clean

test: unit build native

unit:
	$(DOTNET) run --project selftest/SelfTest.csproj --nologo -v q

build:
	$(DOTNET) restore $(PROJECT) --nologo -v q
	$(DOTNET) publish $(PROJECT) $(PUBLISH_FLAGS) -o $(OUT)

native: build
	sh test_probe.sh "$$(command -v $(DOTNET))" $(OUT)/LangAtProbe.dll $(RUNS)

clean:
	rm -rf build src/bin src/obj selftest/bin selftest/obj
