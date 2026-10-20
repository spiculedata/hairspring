package sqlcancel

import "github.com/spiculedata/hairspring/examples/go-sql-cancel/internal/hs"

// Sites are the protocol sites of the workload. Only ScanReturnsOwnRow decides an
// outcome; the others are coverage, a mechanism marker and harness invariants.
type Sites struct {
	ScanReturnsOwnRow    *hs.Handle // always: a Scan that returned nil returned the row of its own query
	ScanErrorAfterCancel *hs.Handle // reachable: Scan returned an error (allowed behaviour)
	CancelOverlappedScan *hs.Handle // sometimes: the cancel flag flipped while Scan ran
	ConnHandedToWaiter   *hs.Handle // sometimes: a Next ran while another query's Scan was in flight
	ErrorWithoutCancel   *hs.Handle // always (harness): an error implies the cancel flag was set
	ConnExclusive        *hs.Handle // always (harness): the pool serialises use of the one connection
	AllQueriesRan        *hs.Handle // always (harness): every query was run and counted
}

// DeclareSites declares the seven sites through declare, which is hs.Declare in the program and a closure over a
// private catalog in tests.
func DeclareSites(declare func(hs.Site) *hs.Handle) *Sites {
	return &Sites{
		ScanReturnsOwnRow: declare(hs.Site{ID: "go.sql.scan_returns_own_row", Kind: hs.KindAlways,
			Msg: "Scan returned a value that is not the row of its own query", Func: "scanOne"}),
		ScanErrorAfterCancel: declare(hs.Site{ID: "go.sql.scan_error_after_cancel", Kind: hs.KindReachable,
			Msg: "Scan returned an error, which a cancelled query may do", Func: "scanOne"}),
		CancelOverlappedScan: declare(hs.Site{ID: "go.sql.cancel_overlapped_scan", Kind: hs.KindSometimes,
			Msg: "the cancel flag flipped while Scan was running", Func: "scanOne"}),
		ConnHandedToWaiter: declare(hs.Site{ID: "go.sql.conn_handed_to_waiter", Kind: hs.KindSometimes,
			Msg: "a query ran Next while an earlier query's Scan was in flight", Func: "onNext"}),
		ErrorWithoutCancel: declare(hs.Site{ID: "go.sql.harness.error_without_cancel", Kind: hs.KindAlways,
			Msg: "an error arrived although the query was not cancelled", Func: "errorSeen"}),
		ConnExclusive: declare(hs.Site{ID: "go.sql.harness.conn_exclusive", Kind: hs.KindAlways,
			Msg: "two queries owned the one connection at once", Func: "onQuery"}),
		AllQueriesRan: declare(hs.Site{ID: "go.sql.harness.all_queries_ran", Kind: hs.KindAlways,
			Msg: "not every query was run and counted", Func: "Run"}),
	}
}
