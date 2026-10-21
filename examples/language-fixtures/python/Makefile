# Python lost-update fixture (buggy, fixed and wide arms in one program), built on the Python SDK in sdks/python.
#
#   make test     the fixture's unit tests, then the native self-check (see ../README.md)
#   make unit     only the unit tests
#   make native   only the native self-check and its negative control
#
# Needs Python 3.12 or newer (PYTHON=python3.12 to choose). Nothing here runs Hermit.

PYTHON ?= python3
RUNS ?= 50
CONTROL_RUNS ?= 10

.PHONY: test unit native

test: unit native

unit:
	$(PYTHON) -m unittest discover -s . -p 'test_*.py'

# Exit 77 means the racy check was skipped on a single-CPU host (the threads cannot overlap): reported, not a failure.
native:
	$(PYTHON) check_native.py --runs $(RUNS) --switch-arm || [ $$? -eq 77 ]
	$(PYTHON) check_native.py --runs $(CONTROL_RUNS) --control || [ $$? -eq 77 ]
