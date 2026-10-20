// Package sqlcancel is the sqlcancel workload: four workers race database/sql query cancellation against Rows.Scan
// on a one-connection pool over a driver that reuses its row buffer (the case fixed upstream in go1.24.6 by
// commit 83b4a5db2409). Toolchains and build hashes are recorded in BUILD-RECORD.
package sqlcancel

import "github.com/spiculedata/hairspring/examples/go-sql-cancel/internal/retaindrv"

// Row classes, in the order the witness reports them.
const (
	ClassOwn        int64 = 0 // exactly the row of the query that scanned it
	ClassForeign    int64 = 1 // a whole row of another query
	ClassTorn       int64 = 2 // head and tail ids differ: the copy straddled a rewrite
	ClassUnparsable int64 = 3 // not a row at all, or a row whose ids are the query's own but is not its row
)

// Verdict is what a scanned string turned out to be. Head and Tail are -1 when the string is unparsable.
type Verdict struct{ Class, Head, Tail int64 }

// Classify compares got with the row that query qid must have received.
func Classify(qid int64, got string) Verdict {
	if got == string(retaindrv.AppendRow(nil, qid, 0)) {
		return Verdict{Class: ClassOwn, Head: qid, Tail: qid}
	}
	r, ok := retaindrv.ParseRow([]byte(got))
	switch {
	case !ok:
		return Verdict{Class: ClassUnparsable, Head: -1, Tail: -1}
	case r.Head != r.Tail:
		return Verdict{Class: ClassTorn, Head: r.Head, Tail: r.Tail}
	case r.Head != qid:
		return Verdict{Class: ClassForeign, Head: r.Head, Tail: r.Tail}
	default:
		return Verdict{Class: ClassUnparsable, Head: -1, Tail: -1}
	}
}
