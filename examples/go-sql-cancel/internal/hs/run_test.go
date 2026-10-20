package hs

import (
	"errors"
	"strings"
	"sync"
	"testing"
)

// sink records every write as one element, so tests can see line boundaries.
type sink struct {
	mu    sync.Mutex
	lines []string
}

func (s *sink) write(b []byte) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.lines = append(s.lines, string(b))
	return nil
}

func (s *sink) kinds(t *testing.T) []string {
	t.Helper()
	var out []string
	for _, l := range s.lines {
		out = append(out, decode(t, []byte(l))["kind"].(string))
	}
	return out
}

func program(t *testing.T) (*Catalog, func() int) {
	c := NewCatalog()
	ok := mustDeclare(t, c, Site{ID: "r.ok", Kind: KindAlways})
	bad := mustDeclare(t, c, Site{ID: "r.bad", Kind: KindAlways, Msg: "went wrong"})
	return c, func() int {
		ok.Check(true, Vals{})
		bad.Check(false, Vals{1, 2, 3, 4})
		c.AddWork(300)
		return 3
	}
}

func TestRunWritesTheFullReportInProtocolOrderAndReturnsTheStatus(t *testing.T) {
	// Arrange
	c, fn := program(t)
	s := &sink{}

	// Act
	status := Run(c, Config{Proc: "p1", Write: s.write}, fn)

	// Assert
	want := []string{"hello", "catalog", "catalog", "assert", "assert", "work", "end"}
	got := s.kinds(t)
	if status != 3 || strings.Join(got, ",") != strings.Join(want, ",") {
		t.Fatalf("status=%d kinds=%v, want 3 and %v", status, got, want)
	}
	end := decode(t, []byte(s.lines[len(s.lines)-1]))
	if end["sites"] != float64(2) || end["evals"] != float64(2) || end["status"] != float64(3) || end["threads"] != float64(0) {
		t.Fatalf("end = %v", end)
	}
}

func TestRunAnnouncesTheCatalogBeforeTheWorkloadStarts(t *testing.T) {
	c := NewCatalog()
	h := mustDeclare(t, c, Site{ID: "r.early", Kind: KindReachable})
	s := &sink{}
	var linesAtStart int

	Run(c, Config{Proc: "p1", Write: s.write}, func() int {
		linesAtStart = len(s.lines)
		h.Hit(Vals{})
		return 0
	})

	if linesAtStart != 2 {
		t.Fatalf("lines written before the workload = %d, want hello and one catalog", linesAtStart)
	}
}

func TestRunFreezesTheCatalogDuringTheWorkload(t *testing.T) {
	c := NewCatalog()
	var declareErr error

	Run(c, Config{Proc: "p1", Write: (&sink{}).write}, func() int {
		_, declareErr = c.Declare(Site{ID: "r.late", Kind: KindAlways})
		return 0
	})

	if declareErr == nil {
		t.Fatal("a site declared after the catalog went out was accepted")
	}
}

func TestRunReportsDroppedStartupWritesInTheEndLine(t *testing.T) {
	c, fn := program(t)
	failures := 0
	var captured []string
	write := func(b []byte) error {
		if failures < 2 { // hello and the first catalog line fail
			failures++
			return errors.New("transient")
		}
		captured = append(captured, string(b))
		return nil
	}

	Run(c, Config{Proc: "p1", Write: write}, fn)

	end := decode(t, []byte(captured[len(captured)-1]))
	if end["dropped"] != float64(2) {
		t.Fatalf("dropped = %v, want 2", end["dropped"])
	}
}

func TestRunDisabledWritesNothingButStillRunsTheWorkload(t *testing.T) {
	c, fn := program(t)
	s := &sink{}

	status := Run(c, Config{Proc: "p1", Write: s.write, Disabled: true}, fn)

	if status != 3 || len(s.lines) != 0 {
		t.Fatalf("status=%d lines=%d, want 3 and 0", status, len(s.lines))
	}
}

func TestRunViolationOnlyWritesOnlyViolationLines(t *testing.T) {
	c, fn := program(t)
	s := &sink{}

	status := Run(c, Config{Proc: "p1", Write: s.write, ViolationOnly: true}, fn)

	if status != 3 {
		t.Fatalf("status = %d, want the workload's 3", status)
	}
	if len(s.lines) != 1 || s.lines[0] != "HAIRSPRING_VIOLATION: r.bad: went wrong (1 false)\n" {
		t.Fatalf("lines = %q", s.lines)
	}
}

func TestRunViolationOnlyIsSilentWhenNothingFailed(t *testing.T) {
	c := NewCatalog()
	h := mustDeclare(t, c, Site{ID: "r.fine", Kind: KindAlways})
	s := &sink{}

	Run(c, Config{Proc: "p1", Write: s.write, ViolationOnly: true}, func() int { h.Check(true, Vals{}); return 0 })

	if len(s.lines) != 0 {
		t.Fatalf("lines = %q, want none", s.lines)
	}
}

func TestConfigFromEnv(t *testing.T) {
	env := func(m map[string]string) func(string) string { return func(k string) string { return m[k] } }
	w := (&sink{}).write

	def := configFromEnv(env(nil), w, false)
	tagged := configFromEnv(env(map[string]string{"HAIRSPRING_PROC": "p1.f2"}), w, false)
	badTag := configFromEnv(env(map[string]string{"HAIRSPRING_PROC": "has space"}), w, false)
	off := configFromEnv(env(map[string]string{"HAIRSPRING_EVENTS": "off"}), w, false)
	viol := configFromEnv(env(nil), w, true)

	if def.Proc != "p1" || tagged.Proc != "p1.f2" || badTag.Proc != "p1" {
		t.Errorf("procs = %q %q %q", def.Proc, tagged.Proc, badTag.Proc)
	}
	if def.Disabled || !off.Disabled || def.ViolationOnly || !viol.ViolationOnly {
		t.Errorf("flags: def=%+v off=%+v viol=%+v", def, off, viol)
	}
}

func TestPackageLevelDeclareAndAddWorkUseTheDefaultCatalog(t *testing.T) {
	saved := std
	std = NewCatalog()
	defer func() { std = saved }()
	h := Declare(Site{ID: "pkg.site", Kind: KindReachable})
	AddWork(4)
	s := &sink{}

	status := Run(std, Config{Proc: "p1", Write: s.write}, func() int { h.Hit(Vals{}); return 0 })

	got := strings.Join(s.kinds(t), ",")
	work := decode(t, []byte(s.lines[len(s.lines)-2]))
	if status != 0 || got != "hello,catalog,assert,work,end" || work["ops"] != float64(4) {
		t.Fatalf("status=%d kinds=%s work=%v", status, got, work)
	}
}

func TestDeclarePanicsOnInvalidSite(t *testing.T) {
	defer func() {
		if recover() == nil {
			t.Fatal("Declare of an invalid site did not panic")
		}
	}()
	Declare(Site{ID: "not valid", Kind: KindAlways})
}
