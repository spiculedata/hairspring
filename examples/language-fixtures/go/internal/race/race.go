// Package race is the Go lost-update fixture: Workers goroutines each do Iterations times
//
//	read the shared counter; spin for Spin rounds of user code (no system call); write back the value read plus one.
//
// The read and the write are separate atomic operations, so the program has no data race in the Go memory model's
// sense, but an update can be lost. Locked is the twin that holds a mutex across all three steps.
package race

import (
	"fmt"
	"hash/fnv"
	"sync"
	"sync/atomic"

	hs "github.com/spiculedata/hairspring/sdks/go"
)

// The fixture's fixed parameters. They are the same for every language (see ../../../README.md).
const (
	Workers    = 4
	Iterations = 1000
	Spin       = 200 // rounds of the busy loop inside the read-modify-write window
	Expected   = Workers * Iterations
)

// Outcome is what one run of the fixture produced.
type Outcome struct {
	Final   uint64
	Started uint32
	Order   [Workers]uint8 // worker ids in the order the workers finished
}

// Racy runs the program with no lock around the read-modify-write.
func Racy() Outcome { return run(noLock{}) }

// Locked runs the same program with a mutex held across the read-modify-write.
func Locked() Outcome { return run(&sync.Mutex{}) }

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
	sinks    [Workers]paddedWord
}

func run(gate sync.Locker) Outcome {
	var st state
	var wg sync.WaitGroup
	for id := uint8(0); id < Workers; id++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			work(id, gate, &st)
		}()
	}
	wg.Wait()
	return Outcome{Final: st.counter.Load(), Started: st.started.Load(), Order: st.order}
}

func work(id uint8, gate sync.Locker, st *state) {
	st.started.Add(1)
	for i := 0; i < Iterations; i++ {
		gate.Lock()
		seen := st.counter.Load()
		pad := spin(seen, Spin) // the window: user code only, no system call
		st.counter.Store(seen + 1)
		gate.Unlock()
		st.sinks[id].v.Store(pad) // keeps the busy loop's result live, so the compiler cannot drop the loop
		hs.AddWork(1)
	}
	st.order[st.arrivals.Add(1)-1] = id
}

// spin is the busy loop: a multiply-add chain that touches no memory and makes no call.
func spin(seed uint64, rounds int) uint64 {
	acc := seed
	for i := 0; i < rounds; i++ {
		acc = acc*6364136223846793005 + 1442695040888963407
	}
	return acc
}

// orderHash is FNV-1a over the arrival order, one byte per worker.
func orderHash(order []uint8) uint64 {
	h := fnv.New64a()
	_, _ = h.Write(order) // a hash's Write never fails
	return h.Sum64()
}

// Line is the one stdout line: fixed width, so its length does not depend on the outcome. It has the same shape as
// the C and C++ fixtures' line.
func Line(lang string, o Outcome, random uint64) string {
	return fmt.Sprintf("lang=%-3s final=%010d order=%016x rand=%016x\n", lang, o.Final, orderHash(o.Order[:]), random)
}
