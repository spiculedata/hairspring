# Language fixtures: a planted lost-update race in C, C++, Go and Python (Rust's is in sdks/rust), one program per
# language with three modes (buggy, fixed, wide). See README.md.
#
#   make test     build and self-check C (with $(CC)), C++ (clang++ unless CXX is given), Go and Python
#   make clean

.PHONY: test test-scripts test-c test-cpp test-go test-python clean

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

clean:
	$(MAKE) -C c clean
	$(MAKE) -C cpp clean
	$(MAKE) -C go clean
