package hs

import (
	"sync"
	"testing"
)

func mustDeclare(t *testing.T, c *Catalog, s Site) *Handle {
	t.Helper()
	h, err := c.Declare(s)
	if err != nil {
		t.Fatalf("Declare(%+v) = %v", s, err)
	}
	return h
}

func siteReport(t *testing.T, c *Catalog, id string) SiteReport {
	t.Helper()
	for _, r := range c.Snapshot().Sites {
		if r.Site.ID == id {
			return r
		}
	}
	t.Fatalf("no site %q in snapshot", id)
	return SiteReport{}
}

func TestCheckCountsTruesAndFalsesAndKeepsFirstWitnesses(t *testing.T) {
	// Arrange
	c := NewCatalog()
	h := mustDeclare(t, c, Site{ID: "a.always", Kind: KindAlways})

	// Act
	h.Check(true, Vals{1, 2, 3, 4})
	h.Check(false, Vals{7, 8, 0, 0})
	h.Check(false, Vals{9, 9, 9, 9})
	h.Check(true, Vals{5, 5, 5, 5})

	// Assert
	r := siteReport(t, c, "a.always")
	if r.Trues != 2 || r.Falses != 2 || r.Hits() != 4 {
		t.Fatalf("trues/falses/hits = %d/%d/%d, want 2/2/4", r.Trues, r.Falses, r.Hits())
	}
	if r.FirstFalse == nil || r.FirstFalse.Vals != (Vals{7, 8, 0, 0}) {
		t.Fatalf("FirstFalse = %+v, want vals 7,8,0,0", r.FirstFalse)
	}
	if r.FirstTrue == nil || r.FirstTrue.Vals != (Vals{1, 2, 3, 4}) || r.FirstTrue.Ord != 1 {
		t.Fatalf("FirstTrue = %+v, want vals 1,2,3,4 ord 1", r.FirstTrue)
	}
	if r.FirstFalse.Ord != 2 {
		t.Fatalf("FirstFalse.Ord = %d, want 2 (second evaluation of the site)", r.FirstFalse.Ord)
	}
}

func TestPointKindsIgnoreTheCondition(t *testing.T) {
	c := NewCatalog()
	reach := mustDeclare(t, c, Site{ID: "p.reach", Kind: KindReachable})
	unreach := mustDeclare(t, c, Site{ID: "p.unreach", Kind: KindUnreachable})

	reach.Check(false, Vals{})
	unreach.Hit(Vals{1, 0, 0, 0})
	unreach.Hit(Vals{2, 0, 0, 0})

	if r := siteReport(t, c, "p.reach"); r.Trues != 1 || r.Falses != 0 || r.FirstFalse != nil {
		t.Fatalf("reachable = %+v, want one true, no false", r)
	}
	if r := siteReport(t, c, "p.unreach"); r.Trues != 2 || r.Falses != 0 || r.FirstTrue.Vals[0] != 1 {
		t.Fatalf("unreachable = %+v, want two hits, first witness 1", r)
	}
}

func TestDeclareMergesSameIDAndKind(t *testing.T) {
	c := NewCatalog()
	h1 := mustDeclare(t, c, Site{ID: "m.shared", Kind: KindSometimes, Line: 1})
	h2 := mustDeclare(t, c, Site{ID: "m.shared", Kind: KindSometimes, Line: 99})

	h1.Check(true, Vals{})
	h2.Check(false, Vals{})

	if h1 != h2 {
		t.Fatal("same id and kind should return the same handle")
	}
	if got := len(c.Snapshot().Sites); got != 1 {
		t.Fatalf("sites = %d, want 1", got)
	}
	if r := siteReport(t, c, "m.shared"); r.Hits() != 2 {
		t.Fatalf("hits = %d, want 2", r.Hits())
	}
}

func TestDeclareRejectsKindConflictInvalidSiteAndFrozenCatalog(t *testing.T) {
	c := NewCatalog()
	mustDeclare(t, c, Site{ID: "k.one", Kind: KindAlways})

	if _, err := c.Declare(Site{ID: "k.one", Kind: KindSometimes}); err == nil {
		t.Error("kind conflict accepted")
	}
	if _, err := c.Declare(Site{ID: "bad id", Kind: KindAlways}); err == nil {
		t.Error("invalid site accepted")
	}
	c.Freeze()
	if _, err := c.Declare(Site{ID: "k.two", Kind: KindAlways}); err == nil {
		t.Error("declare after Freeze accepted")
	}
	if _, err := c.Declare(Site{ID: "k.one", Kind: KindAlways}); err != nil {
		t.Errorf("re-declaring an existing site after Freeze = %v, want it merged", err)
	}
}

func TestSnapshotIsSortedAndDetachedFromLaterUpdates(t *testing.T) {
	c := NewCatalog()
	b := mustDeclare(t, c, Site{ID: "b", Kind: KindAlways})
	mustDeclare(t, c, Site{ID: "a", Kind: KindAlways})
	b.Check(true, Vals{})
	c.AddWork(5)

	before := c.Snapshot()
	b.Check(false, Vals{})
	c.AddWork(2)

	if before.Sites[0].Site.ID != "a" || before.Sites[1].Site.ID != "b" {
		t.Fatalf("snapshot order = %s,%s, want a,b", before.Sites[0].Site.ID, before.Sites[1].Site.ID)
	}
	if before.Sites[1].Falses != 0 || before.Ops != 5 {
		t.Fatalf("snapshot changed after the fact: falses=%d ops=%d", before.Sites[1].Falses, before.Ops)
	}
	if after := c.Snapshot(); after.Ops != 7 || after.Evals() != 2 {
		t.Fatalf("after: ops=%d evals=%d, want 7 and 2", after.Ops, after.Evals())
	}
}

func TestViolationsAreFalseAlwaysAndHitUnreachable(t *testing.T) {
	c := NewCatalog()
	mustDeclare(t, c, Site{ID: "v.ok", Kind: KindAlways}).Check(true, Vals{})
	mustDeclare(t, c, Site{ID: "v.bad", Kind: KindAlways}).Check(false, Vals{})
	mustDeclare(t, c, Site{ID: "v.aou", Kind: KindAlwaysOrUnreachable}).Check(false, Vals{})
	mustDeclare(t, c, Site{ID: "v.unreach", Kind: KindUnreachable}).Hit(Vals{})
	mustDeclare(t, c, Site{ID: "v.unreach.clean", Kind: KindUnreachable})
	mustDeclare(t, c, Site{ID: "v.sometimes", Kind: KindSometimes}).Check(false, Vals{})

	var ids []string
	for _, r := range c.Snapshot().Violations() {
		ids = append(ids, r.Site.ID)
	}

	want := []string{"v.aou", "v.bad", "v.unreach"}
	if len(ids) != len(want) || ids[0] != want[0] || ids[1] != want[1] || ids[2] != want[2] {
		t.Fatalf("violations = %v, want %v", ids, want)
	}
}

func TestConcurrentChecksAreExact(t *testing.T) {
	c := NewCatalog()
	h := mustDeclare(t, c, Site{ID: "race.count", Kind: KindAlways})
	const goroutines, perGoroutine = 8, 5000

	var wg sync.WaitGroup
	for g := 0; g < goroutines; g++ {
		wg.Add(1)
		go func(g int) {
			defer wg.Done()
			for i := 0; i < perGoroutine; i++ {
				h.Check(i%10 != 0, Vals{int64(g), int64(i), 0, 0})
			}
		}(g)
	}
	wg.Wait()

	r := siteReport(t, c, "race.count")
	if r.Hits() != goroutines*perGoroutine || r.Falses != goroutines*perGoroutine/10 {
		t.Fatalf("hits=%d falses=%d", r.Hits(), r.Falses)
	}
	if r.FirstFalse == nil || r.FirstFalse.Vals[1]%10 != 0 {
		t.Fatalf("FirstFalse = %+v, want a witness from a false evaluation", r.FirstFalse)
	}
}
