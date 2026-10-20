package sqlcancel

import (
	"context"
	"database/sql/driver"
	"testing"
	"time"

	"github.com/spiculedata/hairspring/examples/go-sql-cancel/internal/retaindrv"
)

func driverRows(t *testing.T, q driver.QueryerContext, qid int64) driver.Rows {
	t.Helper()
	r, err := q.QueryContext(context.Background(), "q", []driver.NamedValue{{Ordinal: 1, Value: qid}})
	if err != nil {
		t.Fatal(err)
	}
	return r
}

func nextValue(t *testing.T, r driver.Rows) []byte {
	t.Helper()
	dest := make([]driver.Value, 1)
	if err := r.Next(dest); err != nil {
		t.Fatal(err)
	}
	return dest[0].([]byte)
}

func TestYieldScannerCopiesTheWholeRowWhenNothingChangesDuringTheSleep(t *testing.T) {
	var slept []time.Duration
	s := NewYieldScanner(func(d time.Duration) { slept = append(slept, d) }, func() {}, func() {})

	if err := s.Scan([]byte(row(4, 0))); err != nil {
		t.Fatal(err)
	}

	if s.Value() != row(4, 0) {
		t.Fatalf("value = %q", s.Value())
	}
	if len(slept) != 1 || slept[0] != time.Millisecond {
		t.Fatalf("slept %v, want exactly one 1ms sleep", slept)
	}
}

func TestYieldScannerSeesATornRowWhenTheBufferIsRewrittenDuringTheSleep(t *testing.T) {
	conn, _ := retaindrv.NewConnector(retaindrv.Hooks{}).Connect(context.Background())
	q := conn.(driver.QueryerContext)
	a := driverRows(t, q, 1)
	src := nextValue(t, a) // driver-owned bytes of query 1
	_ = a.Close()
	rewrite := func(time.Duration) { nextValue(t, driverRows(t, q, 2)) } // query 2 reuses the connection

	s := NewYieldScanner(rewrite, func() {}, func() {})
	if err := s.Scan(src); err != nil {
		t.Fatal(err)
	}

	v := Classify(1, s.Value())
	if v.Class != ClassTorn || v.Head != 1 || v.Tail != 2 {
		t.Fatalf("verdict %+v for %q", v, s.Value())
	}
}

func TestYieldScannerMarksItselfInFlightAroundTheCopyOnly(t *testing.T) {
	var events []string
	s := NewYieldScanner(
		func(time.Duration) { events = append(events, "sleep") },
		func() { events = append(events, "enter") },
		func() { events = append(events, "leave") },
	)
	_ = s.Scan([]byte(row(1, 0)))
	if got := events; len(got) != 3 || got[0] != "enter" || got[1] != "sleep" || got[2] != "leave" {
		t.Fatalf("events = %v", got)
	}
}

func TestYieldScannerRejectsWhatIsNotARowAndLeavesNothingInFlight(t *testing.T) {
	var entered, left int
	s := NewYieldScanner(func(time.Duration) {}, func() { entered++ }, func() { left++ })
	for _, src := range []any{"a string", []byte("short"), nil, int64(3)} {
		if err := s.Scan(src); err == nil {
			t.Errorf("Scan(%v) should fail", src)
		}
	}
	if entered != 0 || left != 0 {
		t.Fatalf("entered=%d left=%d", entered, left)
	}
}
