package sqlcancel

import (
	"flag"
	"fmt"
	"io"
	"strings"
)

// Mode selects the Scan destination. The same binary serves both.
type Mode string

const (
	ModeStd   Mode = "std"   // *string: arm A, the unassisted workload
	ModeYield Mode = "yield" // a sql.Scanner that copies in two halves around a sleep: arm P, the positive control
)

// Workers is fixed by the design: four workers, four cancellers, one connection.
const Workers = 4

// Default queries per worker (the Q ladder starts at 1000 in mode A; mode P is fixed at 100) and the upper bound.
const (
	DefaultPerWorkerStd   = 1000
	DefaultPerWorkerYield = 100
	MaxPerWorker          = 100_000
)

// Exit statuses: 0 on completion whatever the property result; 3 when a harness invariant failed. A usage error
// is ExitUsage, outside 0..3 so it cannot be mistaken for a Go crash (2) or a Hermit deadlock report (1).
const (
	ExitHarnessFailure = 3
	ExitUsage          = 64
)

// Config says what to run. It is a value; nothing modifies it.
type Config struct {
	Mode      Mode
	PerWorker int // queries per worker; the run issues Workers*PerWorker queries
}

// ParseArgs reads -mode=std|yield and -q (queries per worker; absent means the mode's default, and an explicit value outside 1..MaxPerWorker, 0 included, is rejected).
func ParseArgs(args []string) (Config, error) {
	fs := flag.NewFlagSet("sqlcancel", flag.ContinueOnError)
	fs.SetOutput(io.Discard)
	mode := fs.String("mode", string(ModeStd), "std or yield")
	q := fs.Int("q", 0, "queries per worker (default 1000 for std, 100 for yield)")
	if err := fs.Parse(args); err != nil {
		return Config{}, fmt.Errorf("sqlcancel: %w", err)
	}
	if fs.NArg() != 0 {
		return Config{}, fmt.Errorf("sqlcancel: unexpected argument %q", fs.Arg(0))
	}
	m := Mode(strings.TrimSpace(*mode))
	if m != ModeStd && m != ModeYield {
		return Config{}, fmt.Errorf("sqlcancel: unknown -mode %q (want std or yield)", *mode)
	}
	perWorker := *q
	if isFlagSet(fs, "q") && (perWorker < 1 || perWorker > MaxPerWorker) {
		return Config{}, fmt.Errorf("sqlcancel: -q %d out of range 1..%d", perWorker, MaxPerWorker)
	}
	if !isFlagSet(fs, "q") {
		perWorker = defaultPerWorker(m)
	}
	return Config{Mode: m, PerWorker: perWorker}, nil
}

func defaultPerWorker(m Mode) int {
	if m == ModeYield {
		return DefaultPerWorkerYield
	}
	return DefaultPerWorkerStd
}

func isFlagSet(fs *flag.FlagSet, name string) bool {
	set := false
	fs.Visit(func(f *flag.Flag) { set = set || f.Name == name })
	return set
}
