package sqlcancel

import (
	"context"
	"database/sql"
	"sync"
	"time"

	"github.com/spiculedata/hairspring/examples/go-sql-cancel/internal/hs"
	"github.com/spiculedata/hairspring/examples/go-sql-cancel/internal/retaindrv"
)

// Run executes the workload: Workers workers each issue cfg.PerWorker queries on a one-connection pool, handing
// each query's cancel function to a dedicated canceller goroutine just before calling Scan, with no
// acknowledgement, so the worker and the canceller genuinely race. There is no randomness: ids, counts and order
// are fixed, and all variation between runs comes from the schedule.
func Run(cfg Config, sites *Sites) Result {
	s := newRunState(sites)
	db := sql.OpenDB(retaindrv.NewConnector(s.hooks()))
	db.SetMaxOpenConns(1)
	db.SetMaxIdleConns(1)

	cancelCh := make([]chan *queryState, Workers)
	tallies := make([]tally, Workers)
	var workers, cancellers sync.WaitGroup
	for w := 0; w < Workers; w++ {
		cancelCh[w] = make(chan *queryState)
		cancellers.Add(1)
		go func() {
			defer cancellers.Done()
			cancelLoop(cancelCh[w])
		}()
		workers.Add(1)
		go func() {
			defer workers.Done()
			tallies[w] = s.worker(db, cfg, w, cancelCh[w])
		}()
	}
	workers.Wait()
	for _, ch := range cancelCh {
		close(ch)
	}
	cancellers.Wait()
	_ = db.Close()

	var total tally
	for _, t := range tallies {
		total = total.plus(t)
	}
	want := uint64(Workers * cfg.PerWorker)
	s.invariant(sites.AllQueriesRan, total.queries == want, hs.Vals{int64(total.queries), int64(want)})
	return Result{
		Summary: Summary{
			Mode: cfg.Mode, Queries: total.queries, ScanOK: total.scanOK, ScanErrors: total.scanErrors,
			ForeignHead: total.foreignHead, Torn: total.torn, Unparsable: total.unparsable,
			Overlapped: total.overlapped, ForeignNext: s.foreignNext.Load(),
		},
		HarnessFailures: s.harnessFails.Load(),
	}
}

// cancelLoop is a canceller: set the flag, then cancel.
func cancelLoop(ch <-chan *queryState) {
	for st := range ch {
		st.called.Store(true)
		st.cancel()
	}
}

func (s *runState) worker(db *sql.DB, cfg Config, w int, cancelCh chan<- *queryState) tally {
	var total tally
	for i := 0; i < cfg.PerWorker; i++ {
		qid := int64(w*cfg.PerWorker + i + 1)
		total = total.plus(s.runQuery(db, cfg.Mode, w, qid, cancelCh))
	}
	return total
}

// runQuery is one query: query, advance to the row, hand the cancel function over, scan, check, close.
func (s *runState) runQuery(db *sql.DB, mode Mode, w int, qid int64, cancelCh chan<- *queryState) tally {
	t := tally{queries: 1}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	st := &queryState{cancel: cancel}

	rows, err := db.QueryContext(ctx, "q", qid)
	if err != nil {
		s.errorSeen(st, qid, StageQuery) // this query's context is not cancelled yet: a harness failure
		return t
	}
	defer rows.Close()
	if !rows.Next() {
		s.errorSeen(st, qid, StageNext)
		return t
	}
	cancelCh <- st
	s.scanOne(rows, mode, w, qid, st, &t)
	return t
}

// scanOne calls Scan once and evaluates the outcome. In mode std the worker marks the scan in flight around the
// call (imprecise by a few instructions at each end); in mode yield the scanner marks itself around the copy.
func (s *runState) scanOne(rows *sql.Rows, mode Mode, w int, qid int64, st *queryState, t *tally) {
	before := st.called.Load()
	var got string
	var err error
	if mode == ModeYield {
		sc := NewYieldScanner(time.Sleep, func() { s.inflight.Enter(w, qid) }, func() { s.inflight.Leave(w) })
		err = rows.Scan(sc)
		got = sc.Value()
	} else {
		s.inflight.Enter(w, qid)
		err = rows.Scan(&got)
		s.inflight.Leave(w)
	}
	overlapped := !before && st.called.Load()
	s.sites.CancelOverlappedScan.Check(overlapped, hs.Vals{qid})
	if overlapped {
		t.overlapped++
	}
	if err != nil {
		s.checkScanError(st, qid, err, t)
		return
	}
	s.checkRow(qid, got, t)
}
