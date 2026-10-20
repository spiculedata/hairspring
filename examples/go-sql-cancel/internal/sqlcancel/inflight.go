package sqlcancel

import "sync/atomic"

// Inflight records, per worker, the id of the query whose Scan is running (0 when none). The driver's Next uses it
// to notice a connection handed to a waiting query while an earlier query is still scanning.
type Inflight struct{ slots [Workers]atomic.Int64 }

func (f *Inflight) Enter(worker int, qid int64) { f.slots[worker].Store(qid) }
func (f *Inflight) Leave(worker int)            { f.slots[worker].Store(0) }

// Other returns the id of some in-flight query that is not qid.
func (f *Inflight) Other(qid int64) (int64, bool) {
	for i := range f.slots {
		if v := f.slots[i].Load(); v != 0 && v != qid {
			return v, true
		}
	}
	return 0, false
}
