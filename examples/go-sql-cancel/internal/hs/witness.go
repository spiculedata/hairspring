package hs

import "sync/atomic"

// Vals are the four integers a call site attaches to an evaluation.
type Vals [4]int64

// witnessThread is the thread key sent for every witness: goroutines have no stable identity, so all of them
// share one key, and Ord is the site-wide evaluation number rather than a per-thread ordinal.
const witnessThread = "go"

// Witness is the first evaluation of a site with a given outcome.
type Witness struct {
	Thread string
	Ord    uint64
	Vals   Vals
}

const (
	slotEmpty uint32 = iota
	slotWriting
	slotReady
)

// slot keeps the first witness offered to it. Offering never allocates or blocks: one compare-and-swap claims
// the slot, later offers return at once.
type slot struct {
	state atomic.Uint32
	ord   atomic.Uint64
	vals  [4]atomic.Int64
}

func (s *slot) offer(ord uint64, v Vals) {
	if s.state.Load() != slotEmpty || !s.state.CompareAndSwap(slotEmpty, slotWriting) {
		return
	}
	s.ord.Store(ord)
	for i, x := range v {
		s.vals[i].Store(x)
	}
	s.state.Store(slotReady)
}

// load returns the witness, or nil when none was offered or the offer is still being written.
func (s *slot) load() *Witness {
	if s.state.Load() != slotReady {
		return nil
	}
	w := &Witness{Thread: witnessThread, Ord: s.ord.Load()}
	for i := range w.Vals {
		w.Vals[i] = s.vals[i].Load()
	}
	return w
}
