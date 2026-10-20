package sqlcancel

import (
	"errors"
	"time"

	"github.com/spiculedata/hairspring/examples/go-sql-cancel/internal/retaindrv"
)

const (
	firstHalf  = 12
	yieldPause = time.Millisecond
)

// YieldScanner is the positive control's destination (a sql.Scanner). It copies the first half of the driver-owned
// row, sleeps 1 ms, then copies the second half: the same structure as upstream's TestContextCancelDuringScan,
// whose Scanner blocks mid-Scan. It is assisted and window-widening by design and is never counted as an unassisted hit.
type YieldScanner struct {
	sleep        func(time.Duration)
	enter, leave func()
	out          [retaindrv.RowLen]byte
}

// NewYieldScanner builds a scanner for one Scan call. enter and leave bracket the copy (they mark the scan as in
// flight); sleep is time.Sleep in the workload and a stand-in in tests.
func NewYieldScanner(sleep func(time.Duration), enter, leave func()) *YieldScanner {
	return &YieldScanner{sleep: sleep, enter: enter, leave: leave}
}

var errNotARow = errors.New("sqlcancel: yield scanner wants a 25-byte row")

// Scan implements sql.Scanner.
func (s *YieldScanner) Scan(src any) error {
	b, ok := src.([]byte)
	if !ok || len(b) != retaindrv.RowLen {
		return errNotARow
	}
	s.enter()
	defer s.leave()
	copy(s.out[:firstHalf], b[:firstHalf])
	s.sleep(yieldPause)
	copy(s.out[firstHalf:], b[firstHalf:])
	return nil
}

// Value is the string assembled by Scan.
func (s *YieldScanner) Value() string { return string(s.out[:]) }
