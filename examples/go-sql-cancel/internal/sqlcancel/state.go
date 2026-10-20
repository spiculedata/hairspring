package sqlcancel

import (
	"context"
	"errors"
	"strings"
	"sync/atomic"

	"github.com/spiculedata/hairspring/examples/go-sql-cancel/internal/hs"
	"github.com/spiculedata/hairspring/examples/go-sql-cancel/internal/retaindrv"
)

// Stages of a query at which an error can arrive (the second value of the error-site witness).
const (
	StageQuery int64 = 1
	StageNext  int64 = 2
	StageScan  int64 = 3
)

// Scan error classes (the second value of the scan_error_after_cancel witness).
const (
	ErrClassClosed   int64 = 1 // sql: Rows are closed
	ErrClassCanceled int64 = 2 // context canceled
	ErrClassOther    int64 = 3
)

// queryState is shared by the worker and its canceller. The flag is set before cancel is called, so a set flag
// means "the cancel has been issued or is about to be", never the reverse.
type queryState struct {
	called atomic.Bool
	cancel context.CancelFunc
}

// tally is one worker's private count; the workers' tallies are added after they finish.
type tally struct {
	queries, scanOK, scanErrors, foreignHead, torn, unparsable, overlapped uint64
}

func (a tally) plus(b tally) tally {
	return tally{
		a.queries + b.queries, a.scanOK + b.scanOK, a.scanErrors + b.scanErrors, a.foreignHead + b.foreignHead,
		a.torn + b.torn, a.unparsable + b.unparsable, a.overlapped + b.overlapped,
	}
}

// runState is everything one Run shares between goroutines. All of it is atomics or immutable.
type runState struct {
	sites        *Sites
	inflight     Inflight
	harnessFails atomic.Uint64
	foreignNext  atomic.Uint64
}

func newRunState(sites *Sites) *runState { return &runState{sites: sites} }

// hooks connects the driver to the harness invariants and the mechanism marker.
func (s *runState) hooks() retaindrv.Hooks {
	return retaindrv.Hooks{OnQuery: s.onQuery, OnNext: s.onNext}
}

// invariant evaluates a harness site and counts a failure, which makes the process exit with status 3.
func (s *runState) invariant(h *hs.Handle, ok bool, v hs.Vals) {
	h.Check(ok, v)
	if !ok {
		s.harnessFails.Add(1)
	}
}

func (s *runState) onQuery(qid, prevOwner int64) {
	s.invariant(s.sites.ConnExclusive, prevOwner == 0, hs.Vals{qid, prevOwner})
}

func (s *runState) onNext(qid int64) {
	other, found := s.inflight.Other(qid)
	s.sites.ConnHandedToWaiter.Check(found, hs.Vals{qid, other})
	if found {
		s.foreignNext.Add(1)
	}
}

// errorSeen records an error from QueryContext, Next or Scan: it is allowed only once the cancel flag is set.
func (s *runState) errorSeen(st *queryState, qid, stage int64) {
	s.invariant(s.sites.ErrorWithoutCancel, st.called.Load(), hs.Vals{qid, stage})
}

// checkRow evaluates the property for a Scan that returned nil.
func (s *runState) checkRow(qid int64, got string, t *tally) {
	v := Classify(qid, got)
	s.sites.ScanReturnsOwnRow.Check(v.Class == ClassOwn, hs.Vals{qid, v.Head, v.Tail, v.Class})
	t.scanOK++
	if v.Head >= 0 && v.Head != qid {
		t.foreignHead++
	}
	switch v.Class {
	case ClassTorn:
		t.torn++
	case ClassUnparsable:
		t.unparsable++
	}
}

// checkScanError records a Scan error, which database/sql allows after a cancel.
func (s *runState) checkScanError(st *queryState, qid int64, err error, t *tally) {
	t.scanErrors++
	s.sites.ScanErrorAfterCancel.Hit(hs.Vals{qid, classifyScanError(err)})
	s.errorSeen(st, qid, StageScan)
}

func classifyScanError(err error) int64 {
	switch {
	case strings.Contains(err.Error(), "Rows are closed"):
		return ErrClassClosed
	case errors.Is(err, context.Canceled):
		return ErrClassCanceled
	default:
		return ErrClassOther
	}
}
