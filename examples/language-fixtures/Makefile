# Language fixtures: a planted lost-update race in C, C++, Go and Python (Rust's is in sdks/rust), one program per
# language with three modes (buggy, fixed, wide). See README.md.
#
#   make test     build and self-check C (with $(CC)), C++ (clang++ unless CXX is given), Go and Python
#   make test-java  the Java probe and the Java fixture (needs a JDK 21; not part of `make test`)
#   make clean

.PHONY: test test-scripts test-c test-cpp test-go test-python test-java test-dotnet clean

# test-dotnet needs the .NET 10 SDK and is not part of `make test`.
test: test-scripts test-c test-cpp test-go test-python

# The check scripts themselves, with a fake program.
test-scripts:
	sh scripts/test-scripts.sh

test-c:
	$(MAKE) -C c test

test-cpp:
	$(MAKE) -C cpp test

test-go:
	$(MAKE) -C go test

test-python:
	$(MAKE) -C python test

test-java:
	$(MAKE) -C java test flags at-test

# The BCL-only .NET probe: unit checks, build, native tests.
test-dotnet:
	$(MAKE) -C dotnet test

clean:
	$(MAKE) -C c clean
	$(MAKE) -C cpp clean
	$(MAKE) -C go clean
	$(MAKE) -C java clean
	$(MAKE) -C dotnet clean
