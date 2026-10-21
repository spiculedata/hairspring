# Language fixtures: a planted lost-update race in C, C++ and Go, each with a locked twin. See README.md.
#
#   make test     build and self-check all three (C with $(CC), C++ with clang++ unless CXX is given, Go)
#   make clean

.PHONY: test test-scripts test-c test-cpp test-go clean

test: test-scripts test-c test-cpp test-go

# The check scripts themselves, with a fake program.
test-scripts:
	sh scripts/test-scripts.sh

test-c:
	$(MAKE) -C c test

test-cpp:
	$(MAKE) -C cpp test

test-go:
	$(MAKE) -C go test

clean:
	$(MAKE) -C c clean
	$(MAKE) -C cpp clean
	$(MAKE) -C go clean
