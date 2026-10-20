package sqlcancel

import (
	"testing"

	"github.com/spiculedata/hairspring/examples/go-sql-cancel/internal/hs"
)

// testSites declares the workload's sites in a fresh catalog, so a test never touches the process-wide one.
func testSites(t *testing.T) (*Sites, *hs.Catalog) {
	t.Helper()
	c := hs.NewCatalog()
	s := DeclareSites(func(site hs.Site) *hs.Handle {
		h, err := c.Declare(site)
		if err != nil {
			t.Fatal(err)
		}
		return h
	})
	return s, c
}

func TestDeclareSitesMatchesTheDesignCatalogExactly(t *testing.T) {
	_, c := testSites(t)

	got := map[string]hs.Kind{}
	for _, s := range c.Snapshot().Sites {
		got[s.Site.ID] = s.Site.Kind
	}

	want := map[string]hs.Kind{
		"go.sql.scan_returns_own_row":         hs.KindAlways,
		"go.sql.scan_error_after_cancel":      hs.KindReachable,
		"go.sql.cancel_overlapped_scan":       hs.KindSometimes,
		"go.sql.conn_handed_to_waiter":        hs.KindSometimes,
		"go.sql.harness.error_without_cancel": hs.KindAlways,
		"go.sql.harness.conn_exclusive":       hs.KindAlways,
		"go.sql.harness.all_queries_ran":      hs.KindAlways,
	}
	if len(got) != len(want) {
		t.Fatalf("declared %d sites, want %d: %v", len(got), len(want), got)
	}
	for id, kind := range want {
		if got[id] != kind {
			t.Errorf("site %q: kind %q, want %q", id, got[id], kind)
		}
	}
}

func TestEverySiteIsAValidProtocolSite(t *testing.T) {
	_, c := testSites(t)
	for _, s := range c.Snapshot().Sites {
		if err := s.Site.Validate(); err != nil {
			t.Errorf("%s: %v", s.Site.ID, err)
		}
	}
}

func TestThePropertyViolationMessageIsTheDesignText(t *testing.T) {
	_, c := testSites(t)
	for _, s := range c.Snapshot().Sites {
		if s.Site.ID == "go.sql.scan_returns_own_row" {
			if want := "Scan returned a value that is not the row of its own query"; s.Site.Msg != want {
				t.Fatalf("msg = %q", s.Site.Msg)
			}
			return
		}
	}
	t.Fatal("property site missing")
}
