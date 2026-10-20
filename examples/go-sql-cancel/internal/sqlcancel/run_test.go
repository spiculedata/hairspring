package sqlcancel

import (
	"context"
	"errors"
	"fmt"
	"os"
	"testing"

	"github.com/spiculedata/hairspring/examples/go-sql-cancel/internal/hs"
)

func siteReport(t *testing.T, c *hs.Catalog, id string) hs.SiteReport {
	t.Helper()
	for _, s := range c.Snapshot().Sites {
		if s.Site.ID == id {
			return s
		}
	}
	t.Fatalf("no site %q", id)
	return hs.SiteReport{}
}

const (
	propertyID  = "go.sql.scan_returns_own_row"
	errorSiteID = "go.sql.scan_error_after_cancel"
)

func TestRunAccountsForEveryQueryInBothModes(t *testing.T) {
	for _, mode := range []Mode{ModeStd, ModeYield} {
		t.Run(string(mode), func(t *testing.T) {
			sites, c := testSites(t)

			res := Run(Config{Mode: mode, PerWorker: 10}, sites)

			if res.Queries != Workers*10 {
				t.Fatalf("Queries = %d, want %d", res.Queries, Workers*10)
			}
			if res.ScanOK+res.ScanErrors != res.Queries {
				t.Fatalf("ok %d + errors %d != queries %d", res.ScanOK, res.ScanErrors, res.Queries)
			}
			if res.HarnessFailures != 0 || res.ExitStatus() != 0 {
				t.Fatalf("harness failures: %d", res.HarnessFailures)
			}
			if r := siteReport(t, c, "go.sql.harness.all_queries_ran"); r.Trues != 1 || r.Falses != 0 {
				t.Fatalf("all_queries_ran = %+v", r)
			}
			if r := siteReport(t, c, propertyID); r.Hits() != res.ScanOK {
				t.Fatalf("property evaluated %d times, %d scans succeeded", r.Hits(), res.ScanOK)
			}
			if r := siteReport(t, c, errorSiteID); r.Hits() != res.ScanErrors {
				t.Fatalf("error site hit %d times, %d scans failed", r.Hits(), res.ScanErrors)
			}
		})
	}
}

func TestRunCountersAgreeWithThePropertySiteOnAnyToolchain(t *testing.T) {
	sites, c := testSites(t)

	res := Run(Config{Mode: ModeYield, PerWorker: 20}, sites)

	// Every bad row is foreign (head differs from the query id), torn or unparsable, so a false evaluation
	// implies at least one of the three counters moved, and the counters cannot move without a false one.
	bad := res.ForeignHead + res.Torn + res.Unparsable
	falses := siteReport(t, c, propertyID).Falses
	if (falses > 0) != (bad > 0) {
		t.Fatalf("property false %d times but counters foreign_head=%d torn=%d unparsable=%d",
			falses, res.ForeignHead, res.Torn, res.Unparsable)
	}
}

// SQLCANCEL_EXPECT is set by the toolchain matrix runs: "buggy" for stock go1.24.5, "clean" for go1.24.6 and the
// patched go1.24.5. Mode P (the positive control) must fire on the first and must be exactly zero on the second.
func TestPositiveControlMatchesTheToolchain(t *testing.T) {
	expect := os.Getenv("SQLCANCEL_EXPECT")
	if expect == "" {
		t.Skip("SQLCANCEL_EXPECT not set (buggy|clean)")
	}
	sites, c := testSites(t)

	res := Run(Config{Mode: ModeYield, PerWorker: 100}, sites)

	falses := siteReport(t, c, propertyID).Falses
	t.Logf("%s toolchain: %d false evaluations, %d foreign Next while Scan in flight", expect, falses, res.ForeignNext)
	switch expect {
	case "buggy":
		if falses == 0 {
			t.Fatal("positive control did not fire on a toolchain expected to have the bug")
		}
	case "clean":
		if falses != 0 || res.ForeignNext != 0 {
			t.Fatalf("falses=%d foreign_next=%d on a fixed toolchain", falses, res.ForeignNext)
		}
	default:
		t.Fatalf("SQLCANCEL_EXPECT=%q, want buggy or clean", expect)
	}
}

func TestCheckRowRecordsAForeignRowAsAWitnessedViolation(t *testing.T) {
	sites, c := testSites(t)
	s := newRunState(sites)
	var tl tally

	s.checkRow(3, row(8, 0), &tl)
	s.checkRow(3, row(3, 0), &tl)
	s.checkRow(3, row(3, 0)[:12]+row(8, 0)[12:], &tl)
	s.checkRow(3, "garbage", &tl)

	if tl.scanOK != 4 || tl.foreignHead != 1 || tl.torn != 1 || tl.unparsable != 1 {
		t.Fatalf("tally %+v", tl)
	}
	r := siteReport(t, c, propertyID)
	if r.Trues != 1 || r.Falses != 3 {
		t.Fatalf("property trues=%d falses=%d", r.Trues, r.Falses)
	}
	w := r.FirstFalse
	if w == nil || w.Vals != (hs.Vals{3, 8, 8, ClassForeign}) {
		t.Fatalf("first_false = %+v", w)
	}
}

func TestInvariantFailureCountsAndFlipsTheExitStatus(t *testing.T) {
	sites, c := testSites(t)
	s := newRunState(sites)

	s.invariant(sites.ConnExclusive, true, hs.Vals{1, 0})
	s.invariant(sites.ConnExclusive, false, hs.Vals{2, 1})

	if got := s.harnessFails.Load(); got != 1 {
		t.Fatalf("harness failures = %d, want 1", got)
	}
	if r := siteReport(t, c, "go.sql.harness.conn_exclusive"); r.Falses != 1 {
		t.Fatalf("site falses = %d", r.Falses)
	}
}

func TestOnQueryWithAnotherOwnerIsAHarnessFailure(t *testing.T) {
	sites, _ := testSites(t)
	s := newRunState(sites)

	s.onQuery(5, 0)
	s.onQuery(6, 5)

	if got := s.harnessFails.Load(); got != 1 {
		t.Fatalf("harness failures = %d, want 1", got)
	}
}

func TestOnNextWhileAnotherQueryScansIsTheConnHandoffMarker(t *testing.T) {
	sites, c := testSites(t)
	s := newRunState(sites)

	s.onNext(2) // nothing in flight
	s.inflight.Enter(0, 1)
	s.onNext(2) // query 1 is scanning while query 2 gets rows
	s.onNext(1) // its own query: not foreign

	r := siteReport(t, c, "go.sql.conn_handed_to_waiter")
	if r.Trues != 1 || r.Falses != 2 {
		t.Fatalf("trues=%d falses=%d", r.Trues, r.Falses)
	}
	if r.FirstTrue.Vals != (hs.Vals{2, 1, 0, 0}) {
		t.Fatalf("witness %+v", r.FirstTrue.Vals)
	}
	if s.foreignNext.Load() != 1 {
		t.Fatalf("foreignNext = %d", s.foreignNext.Load())
	}
}

func TestClassifyScanError(t *testing.T) {
	for _, c := range []struct {
		err  error
		want int64
	}{
		{errors.New("sql: Rows are closed"), ErrClassClosed},
		{context.Canceled, ErrClassCanceled},
		{fmt.Errorf("wrapped: %w", context.Canceled), ErrClassCanceled},
		{errors.New("boom"), ErrClassOther},
	} {
		if got := classifyScanError(c.err); got != c.want {
			t.Errorf("classifyScanError(%v) = %d, want %d", c.err, got, c.want)
		}
	}
}

func TestErrorWithoutCancelIsAHarnessFailureOnlyWhenTheCancelFlagIsUnset(t *testing.T) {
	sites, _ := testSites(t)
	s := newRunState(sites)
	st := &queryState{}

	s.errorSeen(st, 1, StageScan) // flag unset: failure
	st.called.Store(true)
	s.errorSeen(st, 2, StageScan) // flag set: allowed

	if got := s.harnessFails.Load(); got != 1 {
		t.Fatalf("harness failures = %d, want 1", got)
	}
}
