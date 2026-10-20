// Package retaindrv is a fake database/sql driver for the sqlcancel workload. Its rows are driver-owned memory:
// every row of every query on a connection is written into one buffer per connection, and Next hands out a slice
// of that buffer, not a copy. That is allowed by the driver.Rows contract (the data is valid only until the next
// Next or Close) and it is exactly the case the upstream fix 83b4a5db2409 describes.
package retaindrv

// Row layout: q<id:08d>-r<row:04d>-q<id:08d>. The id appears at both ends so a copy that straddles a rewrite
// (head from one query, tail from another) is visible.
const (
	RowLen = 25

	// MaxQueryID and MaxRowIndex are the largest values that fit the fixed-width digit fields.
	MaxQueryID  = 99_999_999
	MaxRowIndex = 9_999

	idDigits  = 8
	rowDigits = 4
	headEnd   = 1 + idDigits             // "q" + id
	rowStart  = headEnd + 2              // "-r"
	tailStart = rowStart + rowDigits + 1 // "-"
)

// AppendRow appends the row for (qid, row) to dst and returns the extended slice. It does not allocate when dst
// has room for RowLen more bytes. It panics if qid or row is outside the fixed-width range: that is a bug in the
// caller, and the callers validate their arguments first.
func AppendRow(dst []byte, qid int64, row int) []byte {
	if qid < 0 || qid > MaxQueryID || row < 0 || row > MaxRowIndex {
		panic("retaindrv: row field out of range")
	}
	dst = append(dst, 'q')
	dst = appendDigits(dst, qid, idDigits)
	dst = append(dst, '-', 'r')
	dst = appendDigits(dst, int64(row), rowDigits)
	dst = append(dst, '-', 'q')
	return appendDigits(dst, qid, idDigits)
}

func appendDigits(dst []byte, n int64, width int) []byte {
	start := len(dst)
	for i := 0; i < width; i++ {
		dst = append(dst, '0')
	}
	for i := width - 1; i >= 0 && n > 0; i-- {
		dst[start+i] = byte('0' + n%10)
		n /= 10
	}
	return dst
}

// Row is a parsed row: the two ids and the row index.
type Row struct{ Head, Index, Tail int64 }

// ParseRow reads b as a row. ok is false unless b has exactly the fixed layout. A torn row (Head != Tail) parses.
func ParseRow(b []byte) (r Row, ok bool) {
	if len(b) != RowLen || b[0] != 'q' || b[headEnd] != '-' || b[headEnd+1] != 'r' ||
		b[tailStart-1] != '-' || b[tailStart] != 'q' {
		return Row{}, false
	}
	var h, i, t bool
	r.Head, h = parseDigits(b[1:headEnd])
	r.Index, i = parseDigits(b[rowStart : rowStart+rowDigits])
	r.Tail, t = parseDigits(b[tailStart+1:])
	if !(h && i && t) {
		return Row{}, false
	}
	return r, true
}

func parseDigits(b []byte) (int64, bool) {
	var n int64
	for _, c := range b {
		if c < '0' || c > '9' {
			return 0, false
		}
		n = n*10 + int64(c-'0')
	}
	return n, true
}
