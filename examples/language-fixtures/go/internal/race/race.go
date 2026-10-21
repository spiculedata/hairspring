// Package race is the Go lost-update fixture: Workers goroutines each do Iterations times
//
//	read the shared counter; spin for a number of rounds of user code (no system call); write back the value read plus one.
//
// The read and the write are separate atomic operations, so the program has no data race in the Go memory model's
// sense, but an update can be lost. One program serves three modes chosen by its first argument: buggy (no lock),
// fixed (a mutex held across the three steps) and wide (buggy with a busy loop ten times longer).
package race

import (
	"errors"
	"fmt"
	"hash/fnv"
	"runtime"
	"sync"
	"sync/atomic"

	hs "github.com/spiculedata/hairspring/sdks/go"
)

// The fixture's fixed parameters. They are the same for every language (see ../../../README.md).
const (
	Workers    = 4
	Iterations = 1000
	Spin       = 1000  // rounds of the busy loop inside the read-modify-write window
	SpinWide   = 10000 // the same, in wide mode
	Expected   = Workers * Iterations
)

// Mode is one of the program's three modes. A Mode is a value: choosing a mode never changes another.
type Mode struct {
	Name   string
	Locked bool // a mutex is held across the read-modify-write
	Spin   int  // rounds of the busy loop inside the window
}

// ErrUsage is returned by ParseMode for every argument list that is not exactly one known mode.
var ErrUsage = errors.New("usage: lost_update buggy|fixed|wide")

// ParseMode reads the program's arguments (without the program name): exactly one of buggy, fixed or wide.
func ParseMode(args []string) (Mode, error) {
	if len(args) != 1 {
		return Mode{}, ErrUsage
	}
	switch args[0] {
	case "buggy":
		return Mode{Name: "buggy", Locked: false, Spin: Spin}, nil
	case "fixed":
		return Mode{Name: "fixed", Locked: true, Spin: Spin}, nil
	case "wide":
		return Mode{Name: "wide", Locked: false, Spin: SpinWide}, nil
	}
	return Mode{}, ErrUsage
}

// Outcome is what one run of the fixture produced.
type Outcome struct {
	Final   uint64
	Started uint32
	Arrived uint32         // how many workers recorded their finish; Workers unless an invariant failed
	Order   [Workers]uint8 // the workers' ids as ASCII digits, in the order the workers finished

	// StartedAtFirstIteration is, for each worker, the number of started workers it saw when it passed the start
	// barrier. The barrier holds when every entry is Workers.
	StartedAtFirstIteration [Workers]uint32
}

type noLock struct{}

func (noLock) Lock()   {}
func (noLock) Unlock() {}

// paddedWord keeps each worker's sink on its own cache line, so the sinks do not slow the workers into step.
type paddedWord struct {
	v atomic.Uint64
	_ [56]byte
}

type state struct {
	counter  atomic.Uint64
	started  atomic.Uint32
	arrivals atomic.Uint32
	order    [Workers]uint8
	seen     [Workers]uint32
	sinks    [Workers]paddedWord
}

// Run executes the program once in the given mode.
func Run(mode Mode) Outcome {
	var gate sync.Locker = noLock{}
	if mode.Locked {
		gate = &sync.Mutex{}
	}
	var st state
	var wg sync.WaitGroup
	for id := uint8(0); id < Workers; id++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			work(id, mode.Spin, gate, &st)
		}()
	}
	wg.Wait()
	return Outcome{
		Final:                   st.counter.Load(),
		Started:                 st.started.Load(),
		Arrived:                 st.arrivals.Load(),
		Order:                   st.order,
		StartedAtFirstIteration: st.seen,
	}
}

// work is one worker. Where the busy loop sits relies on the gc compiler's behaviour: it keeps the loop between the
// counter's load and store because the loop's result feeds a later atomic store and gc moves no computation across
// atomic operations. Nothing in the language promises that. If a toolchain ever dropped or moved the loop, the window
// would shrink and TestBuggyLosesAnUpdateAtLeastOnce and the detector test in race_test.go would stop finding lost
// updates, which is how a regression would show.
func work(id uint8, rounds int, gate sync.Locker, st *state) {
	st.started.Add(1)
	for st.started.Load() < Workers { // the start barrier: nobody iterates before everybody has started
		runtime.Gosched()
	}
	st.seen[id] = st.started.Load()
	for i := 0; i < Iterations; i++ {
		gate.Lock()
		seen := st.counter.Load()
		pad := spin(seen, rounds) // the window: user code only, no system call
		st.counter.Store(seen + 1)
		gate.Unlock()
		st.sinks[id].v.Store(pad) // keeps the busy loop's result live, so the compiler cannot drop the loop
		hs.AddWork(1)
	}
	st.order[st.arrivals.Add(1)-1] = '0' + id
}

// spin is the busy loop: a multiply-add chain that touches no memory and makes no call. It is kept out of line so the
// window is one call between the counter's load and store.
//
//go:noinline
func spin(seed uint64, rounds int) uint64 {
	acc := seed
	for i := 0; i < rounds; i++ {
		acc = acc*6364136223846793005 + 1442695040888963407
	}
	return acc
}

// orderHash is FNV-1a over the arrival list written as ASCII digits.
func orderHash(order []uint8) uint64 {
	h := fnv.New64a()
	_, _ = h.Write(order) // a hash's Write never fails
	return h.Sum64()
}

// Line is the one stdout line: fixed width, so its length does not depend on the outcome.
func Line(lang string, mode Mode, o Outcome, random uint64) string {
	return fmt.Sprintf("LANGAT v1 lang=%s mode=%s final=%020d expect=%020d order=%016x rand=%016x\n",
		lang, mode.Name, o.Final, uint64(Expected), orderHash(o.Order[:]), random)
}
