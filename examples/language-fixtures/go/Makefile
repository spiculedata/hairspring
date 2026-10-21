# Go lost-update fixture (racy program and locked twin), built with the Go SDK in sdks/go.
#
#   make              build both programs
#   make test         go vet, go test, then the native self-checks (see ../README.md)
#   make clean
#
# Needs Go 1.24 or newer (GOTOOLCHAIN=local, no network). The programs are static (CGO_ENABLED=0) and are meant to
# run with GOMAXPROCS=2, which `make test` sets. Nothing here runs Hermit.

GO ?= go
BUILD ?= build
RUNS ?= 50
GOFLAGS_BUILD := -trimpath -buildvcs=false

export GOTOOLCHAIN := local
export GOPROXY := off
export CGO_ENABLED := 0

PROGRAMS := $(BUILD)/lost_update_racy $(BUILD)/lost_update_locked

.PHONY: all test clean FORCE

all: $(PROGRAMS)

# go build decides whether anything changed.
$(BUILD)/lost_update_racy: FORCE
	$(GO) build $(GOFLAGS_BUILD) -o $@ .

$(BUILD)/lost_update_locked: FORCE
	$(GO) build $(GOFLAGS_BUILD) -tags locked -o $@ .

FORCE:

# The negative control feeds the racy program in as the "locked twin": the check must reject it, or it checks nothing.
test: $(PROGRAMS)
	$(GO) vet ./...
	$(GO) vet -tags locked ./...
	GOMAXPROCS=2 $(GO) test -count=1 ./...
	GOMAXPROCS=2 sh ../scripts/run-checks.sh go $(BUILD)/lost_update_racy $(BUILD)/lost_update_locked $(RUNS)

clean:
	rm -rf $(BUILD)
