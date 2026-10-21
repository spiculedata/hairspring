# Go lost-update fixture: one program, at-go, with the modes buggy, fixed and wide, built with the Go SDK in sdks/go.
#
#   make              build at-go for this machine
#   make test         go vet, go test, then the native self-checks (see ../README.md)
#   make test-race    go test -race (needs cgo)
#   make release      build the linux/amd64 artifact the acceptance run pins, into build/linux-amd64/at-go
#   make clean
#
# Needs Go 1.24 or newer (GOTOOLCHAIN=local, no network). The program is static (CGO_ENABLED=0) and is meant to
# run with GOMAXPROCS=2, which `make test` sets. Nothing here runs Hermit.

GO ?= go
BUILD ?= build
RUNS ?= 50
GOFLAGS_BUILD := -trimpath -buildvcs=false -ldflags=-buildid=

export GOTOOLCHAIN := local
export GOPROXY := off
export CGO_ENABLED := 0

PROGRAM := $(BUILD)/at-go
RELEASE := $(BUILD)/linux-amd64/at-go

.PHONY: all test test-race release clean FORCE

all: $(PROGRAM)

# go build decides whether anything changed.
$(PROGRAM): FORCE
	$(GO) build $(GOFLAGS_BUILD) -o $@ .

# The artifact of the build record: the same flags, for the box's platform (GOAMD64=v1 is the baseline x86-64).
$(RELEASE): FORCE
	GOOS=linux GOARCH=amd64 GOAMD64=v1 $(GO) build $(GOFLAGS_BUILD) -o $@ .

release: $(RELEASE)

FORCE:

# The negative control runs the buggy mode in place of fixed: the check must reject it, or it checks nothing.
test: $(PROGRAM)
	$(GO) vet ./...
	GOMAXPROCS=2 $(GO) test -count=1 ./...
	sh ../scripts/check-sources.sh main.go internal/race/race.go
	GOMAXPROCS=2 sh ../scripts/run-checks.sh go $(PROGRAM) $(RUNS)

# The race detector needs cgo, unlike the pinned artifact. The fixture has no data race in the Go sense, so this
# must pass: the lost update is a logic race on atomics.
test-race:
	CGO_ENABLED=1 GOMAXPROCS=2 $(GO) test -race -count=1 ./...

clean:
	rm -rf $(BUILD)
