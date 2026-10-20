package hairspring

import (
	"fmt"
	"sort"
	"sync"
	"sync/atomic"
)

// Handle is one declared site's counters. Call it from any goroutine: Check and Hit are a few atomic
// operations, with no system call, lock or allocation.
type Handle struct {
	site                  Site
	evals, trues, falses  atomic.Uint64
	firstTrue, firstFalse slot
}

// Check records one evaluation of an always, always_or_unreachable or sometimes site. On a reachable or
// unreachable site the condition is ignored and the evaluation counts as a hit.
func (h *Handle) Check(ok bool, v Vals) {
	ord := h.evals.Add(1)
	if ok || h.site.Kind.isPoint() {
		h.trues.Add(1)
		h.firstTrue.offer(ord, v)
		return
	}
	h.falses.Add(1)
	h.firstFalse.offer(ord, v)
}

// Hit records that a reachable or unreachable site was reached.
func (h *Handle) Hit(v Vals) { h.Check(true, v) }

// SiteReport is an immutable copy of one site's counters.
type SiteReport struct {
	Site                  Site
	Trues, Falses         uint64
	FirstTrue, FirstFalse *Witness
}

// Hits is trues plus falses, so the protocol's hits == trues + falses rule holds by construction.
func (r SiteReport) Hits() uint64 { return r.Trues + r.Falses }

// Report is an immutable copy of a catalog, sites sorted by id.
type Report struct {
	Sites []SiteReport
	Ops   uint64
}

// Evals is the total number of evaluations over all sites.
func (r Report) Evals() uint64 {
	var n uint64
	for _, s := range r.Sites {
		n += s.Hits()
	}
	return n
}

// Violations are the sites whose outcome is a finding: an always (or always_or_unreachable) that was false, and
// an unreachable that was hit.
func (r Report) Violations() []SiteReport {
	var out []SiteReport
	for _, s := range r.Sites {
		alwaysFalse := s.Falses > 0 && s.Site.Kind != KindSometimes
		if alwaysFalse || (s.Site.Kind == KindUnreachable && s.Trues > 0) {
			out = append(out, s)
		}
	}
	return out
}

// Catalog is the set of declared sites of one process. Declare before the workload starts; Freeze (done by Run)
// then makes the set final, because the catalog lines have already been sent.
type Catalog struct {
	mu      sync.Mutex
	handles map[string]*Handle
	frozen  bool
	ops     atomic.Uint64
}

// NewCatalog returns an empty catalog. Most programs use the process-wide one through Declare, Always and Main.
func NewCatalog() *Catalog { return &Catalog{handles: map[string]*Handle{}} }

// Declare adds a site. The same id with the same kind returns the existing handle (call sites that were
// duplicated merge into one id); the same id with another kind, an invalid site, or a frozen catalog is an error.
func (c *Catalog) Declare(s Site) (*Handle, error) {
	if err := s.Validate(); err != nil {
		return nil, err
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if h, ok := c.handles[s.ID]; ok {
		if h.site.Kind != s.Kind {
			return nil, fmt.Errorf("hairspring: site %q declared as %s and as %s", s.ID, h.site.Kind, s.Kind)
		}
		return h, nil
	}
	if c.frozen {
		return nil, fmt.Errorf("hairspring: site %q declared after the catalog was sent", s.ID)
	}
	h := &Handle{site: s}
	c.handles[s.ID] = h
	return h, nil
}

// Freeze stops new declarations.
func (c *Catalog) Freeze() {
	c.mu.Lock()
	c.frozen = true
	c.mu.Unlock()
}

// AddWork adds to the count of operations reported in the work line.
func (c *Catalog) AddWork(n uint64) { c.ops.Add(n) }

// Snapshot copies the counters. It is meant for the end of the run, once the workload has stopped.
func (c *Catalog) Snapshot() Report {
	c.mu.Lock()
	defer c.mu.Unlock()
	r := Report{Ops: c.ops.Load(), Sites: make([]SiteReport, 0, len(c.handles))}
	for _, h := range c.handles {
		r.Sites = append(r.Sites, SiteReport{
			Site: h.site, Trues: h.trues.Load(), Falses: h.falses.Load(),
			FirstTrue: h.firstTrue.load(), FirstFalse: h.firstFalse.load(),
		})
	}
	sort.Slice(r.Sites, func(i, j int) bool { return r.Sites[i].Site.ID < r.Sites[j].Site.ID })
	return r
}
