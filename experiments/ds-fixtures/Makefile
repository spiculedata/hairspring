# DS experiment fixtures: small C, Go and Rust programs for the multi-process loopback and kill-and-restart experiments.
# Experiment fixtures, not product features. Nothing here runs Hermit; see README.md.
#
#   make               build the C, Go and Rust programs into build/
#   make test          build, then every native self-test (C, Go, Rust, the rule checks, topologies, kills)
#   make test-c        the C programs only: golden files, topologies c3e c3t c3t2 c3b, crash topologies
#   make test-go       go vet, go test, and the g3 topology and the Go node
#   make test-rust     cargo test, and the r3 topology
#   make test-rules    grep-style checks of the fixture rules
#   make test-kills    the native kill stress (KILLS=1000 for the full Linux run; default 100)
#   make clean         remove build outputs
#
# Variables: CC (default cc), OPT, RUNS (topology repetitions, default 20), KILLS, DS_PORT_BASE (default 17100).

CC ?= cc
OPT ?= -O2
RUNS ?= 20
KILLS ?= 100
GO_KILLS ?= 40
DS_PORT_BASE ?= 17100
PYTHON ?= python3

BUILD := build
CFLAGS += -std=gnu11 $(OPT) -g -Wall -Wextra -pthread
LDFLAGS += -pthread
C_PROGRAMS := dsl dskv dscli dsnode
C_BINS := $(addprefix $(BUILD)/,$(C_PROGRAMS))

export DS_BIN := $(abspath $(BUILD))
export DS_PORT_BASE
export DS_RUNS := $(RUNS)
export DS_KILLS := $(KILLS)
export DS_GO_KILLS := $(GO_KILLS)

.PHONY: all c go rust test test-c test-go test-rust test-rules test-kills clean

all: c go rust

c: $(C_BINS)

$(BUILD)/%: c/%.c c/ds_common.h
	mkdir -p $(BUILD)
	$(CC) $(CFLAGS) -o $@ $< $(LDFLAGS)

# Static, no cgo: the same build the experiment's CI lane uses.
go:
	mkdir -p $(BUILD)
	cd go && CGO_ENABLED=0 go build -trimpath -o ../$(BUILD)/dsgo ./cmd/dsgo
	cd go && CGO_ENABLED=0 go build -trimpath -o ../$(BUILD)/dsgonode ./cmd/dsgonode

rust:
	mkdir -p $(BUILD)
	cd rust && cargo build --release --locked
	cp rust/target/release/dsrs $(BUILD)/dsrs

test: test-rules test-c test-go test-rust test-kills

test-rules:
	$(PYTHON) tests/check_rules.py

test-c: $(C_BINS)
	$(PYTHON) tests/test_c.py

test-go: c go
	cd go && go vet ./... && go test ./...
	$(PYTHON) tests/test_go.py

test-rust: c rust
	cd rust && cargo test --locked
	$(PYTHON) tests/test_rust.py

test-kills: c go
	$(PYTHON) tests/test_kills.py

clean:
	rm -rf $(BUILD) rust/target tests/__pycache__
