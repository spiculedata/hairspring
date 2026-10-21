package hairspring

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
)

const goldenDir = "../conformance/goldens"

func goldenLines(t *testing.T, name string) []string {
	t.Helper()
	raw, err := os.ReadFile(filepath.Join(goldenDir, name))
	if err != nil {
		t.Fatalf("reading golden %s: %v", name, err)
	}
	lines, err := splitLines(raw)
	if err != nil {
		t.Fatalf("golden %s: %v", name, err)
	}
	return lines
}

// The checker is only a fair judge of the SDK if it agrees with the shared goldens: it must accept every line the
// reference parser accepts and reject every line it rejects.

func TestCheckerAcceptsEveryLineOfTheValidGoldens(t *testing.T) {
	for _, name := range []string{"valid.events", "ignored.events", "many_threads.events", "merged_sites.events", "unstable_tag.events"} {
		for i, l := range goldenLines(t, name) {
			if _, err := checkLine(l); err != nil {
				t.Errorf("%s line %d: %v\n%s", name, i+1, err, l)
			}
		}
	}
}

func TestCheckerRejectsEveryLineOfTheInvalidGolden(t *testing.T) {
	lines := goldenLines(t, "invalid.events")
	if len(lines) < 20 {
		t.Fatalf("invalid.events has %d lines, expected the full set", len(lines))
	}
	for i, l := range lines {
		if _, err := checkLine(l); err == nil {
			t.Errorf("invalid.events line %d was accepted:\n%s", i+1, l)
		}
	}
}

func TestCheckerAcceptsTheValidGoldenAsOneTrial(t *testing.T) {
	trial := goldenLines(t, "valid.events")[:11] // the 12th line is a script `eval`, which is not part of a process report

	if err := checkTrial(trial); err != nil {
		t.Fatal(err)
	}
}

func TestCheckerCatchesTrialLevelContradictions(t *testing.T) {
	good := reportOf(t, "p1", func(c *Catalog) func() int {
		h := mustDeclare(t, c, Site{ID: "x.one", Kind: KindAlways})
		return func() int { h.Check(true, Vals{}); return 0 }
	})
	if err := checkTrial(good); err != nil {
		t.Fatalf("baseline report rejected: %v", err)
	}
	// good is hello, catalog, assert, work, end.
	const catalog, assert, end = 1, 2, 4
	cases := map[string][]string{
		"no end":              without(good, end),
		"no hello":            without(good, 0),
		"assert dropped":      without(good, assert),
		"duplicate assert":    repeated(good, assert),
		"duplicate catalog":   repeated(good, catalog),
		"another process":     replaced(good, assert, strings.Replace(good[assert], `"p1"`, `"p2"`, 1)),
		"end counts a thread": replaced(good, end, strings.Replace(good[end], `"threads":0`, `"threads":1`, 1)),
	}
	for name, lines := range cases {
		if err := checkTrial(lines); err == nil {
			t.Errorf("%s: accepted", name)
		}
	}
}

// without, repeated and replaced return a new slice and leave their input alone.
func without(lines []string, i int) []string {
	return append(append([]string{}, lines[:i]...), lines[i+1:]...)
}

func repeated(lines []string, i int) []string {
	return append(append([]string{}, lines[:i+1]...), lines[i:]...)
}

func replaced(lines []string, i int, line string) []string {
	out := append([]string{}, lines...)
	out[i] = line
	return out
}

// reportOf runs a program and returns the lines the SDK wrote, each exactly as one write call delivered it.
func reportOf(t *testing.T, proc string, build func(*Catalog) func() int) []string {
	t.Helper()
	c := NewCatalog()
	fn := build(c)
	s := &sink{}
	Run(c, Config{Proc: proc, Write: s.write}, fn)
	out := make([]string, len(s.lines))
	for i, l := range s.lines {
		if strings.Count(l, "\n") != 1 || !strings.HasSuffix(l, "\n") {
			t.Fatalf("write %d is not exactly one line: %q", i, l)
		}
		if len(l) > maxLineBytes {
			t.Fatalf("write %d is %d bytes, over the %d cap", i, len(l), maxLineBytes)
		}
		out[i] = strings.TrimSuffix(l, "\n")
	}
	return out
}

func TestEveryKindOfSiteProducesAConformingReport(t *testing.T) {
	lines := reportOf(t, "p1", func(c *Catalog) func() int {
		always := mustDeclare(t, c, Site{ID: "k.always", Kind: KindAlways, Msg: "m", File: "f.go", Line: 3, Func: "fn"})
		aou := mustDeclare(t, c, Site{ID: "k.aou", Kind: KindAlwaysOrUnreachable})
		sometimes := mustDeclare(t, c, Site{ID: "k.sometimes", Kind: KindSometimes})
		reach := mustDeclare(t, c, Site{ID: "k.reach", Kind: KindReachable})
		unreach := mustDeclare(t, c, Site{ID: "k.unreach", Kind: KindUnreachable})
		mustDeclare(t, c, Site{ID: "k.never", Kind: KindReachable})
		return func() int {
			always.Check(false, Vals{1, 2, 3, 4})
			always.Check(true, Vals{-1, -2, -3, -4})
			aou.Check(true, Vals{})
			sometimes.Check(false, Vals{})
			reach.Hit(Vals{9, 0, 0, 0})
			unreach.Hit(Vals{})
			c.AddWork(77)
			return 1
		}
	})

	if err := checkTrial(lines); err != nil {
		t.Fatal(err)
	}
}

func TestAReportOfManyIDsConforms(t *testing.T) {
	lines := reportOf(t, "p1", func(c *Catalog) func() int {
		var hs []*Handle
		for i := 0; i < 500; i++ {
			hs = append(hs, mustDeclare(t, c, Site{ID: fmt.Sprintf("many.%04d", i), Kind: KindAlways, Msg: "one of many"}))
		}
		return func() int {
			for _, h := range hs {
				h.Check(true, Vals{})
			}
			return 0
		}
	})

	if err := checkTrial(lines); err != nil {
		t.Fatal(err)
	}
	if len(lines) != 1+500+500+1+1 {
		t.Fatalf("lines = %d", len(lines))
	}
}

// The parser counts at most 4096 ids per process, so end.sites says 4096 even when more were declared; the extra
// catalog and assert lines are still sent and the parser marks the trial degraded.
func TestEndSitesIsCappedAtTheProtocolLimit(t *testing.T) {
	const declared = maxSitesPerProcess + 4
	lines := reportOf(t, "p1", func(c *Catalog) func() int {
		for i := 0; i < declared; i++ {
			mustDeclare(t, c, Site{ID: fmt.Sprintf("cap.%05d", i), Kind: KindReachable})
		}
		return func() int { return 0 }
	})

	end := lines[len(lines)-1]
	if len(lines) != 1+2*declared+1+1 || !strings.Contains(end, fmt.Sprintf(`"sites":%d,`, maxSitesPerProcess)) {
		t.Fatalf("lines=%d end=%s", len(lines), end)
	}
}

func TestSitesAtTheProtocolLimitsConform(t *testing.T) {
	lines := reportOf(t, "p1", func(c *Catalog) func() int {
		h := mustDeclare(t, c, Site{
			ID: strings.Repeat("i", 128), Kind: KindAlways,
			Msg: strings.Repeat("<", 512), File: strings.Repeat("<", 256), Line: 1<<64 - 1, Func: strings.Repeat("f", 128),
		})
		return func() int { h.Check(true, Vals{-1 << 63, 1<<63 - 1, 0, 0}); return 0 }
	})

	if err := checkTrial(lines); err != nil {
		t.Fatal(err)
	}
}

func TestAProcessTagFromTheEnvironmentConforms(t *testing.T) {
	lines := reportOf(t, "p1.f3", func(c *Catalog) func() int {
		h := mustDeclare(t, c, Site{ID: "tag.site", Kind: KindReachable})
		return func() int { h.Hit(Vals{}); return 0 }
	})

	if err := checkTrial(lines); err != nil {
		t.Fatal(err)
	}
}

func TestAReportFromConcurrentGoroutinesConformsAndCountsExactly(t *testing.T) {
	lines := reportOf(t, "p1", func(c *Catalog) func() int {
		h := mustDeclare(t, c, Site{ID: "conc.site", Kind: KindAlways})
		return func() int {
			var wg sync.WaitGroup
			for g := 0; g < 4; g++ {
				wg.Add(1)
				go func() {
					defer wg.Done()
					for i := 0; i < 1000; i++ {
						h.Check(i != 999, Vals{int64(i)})
						c.AddWork(1)
					}
				}()
			}
			wg.Wait()
			return 0
		}
	})

	if err := checkTrial(lines); err != nil {
		t.Fatal(err)
	}
	assertLineText := lines[2]
	wantHits, wantFalses := fmt.Sprintf(`"hits":%*d`, numWidth, 4000), fmt.Sprintf(`"falses":%*d`, numWidth, 4)
	if !strings.Contains(assertLineText, wantHits) || !strings.Contains(assertLineText, wantFalses) {
		t.Fatalf("assert line = %s", assertLineText)
	}
}

// The reason for the padding: the report's bytes, and so the arguments of its write calls, must not depend on how
// the program went, or the report itself would change the schedule it is describing.
func TestTheReportsLinesHaveTheSameLengthWhateverTheOutcome(t *testing.T) {
	build := func(ok bool, n int) []string {
		return reportOf(t, "p1", func(c *Catalog) func() int {
			a := mustDeclare(t, c, Site{ID: "len.always", Kind: KindAlways})
			r := mustDeclare(t, c, Site{ID: "len.reach", Kind: KindReachable})
			return func() int {
				for i := 0; i < n; i++ {
					a.Check(ok, Vals{int64(i), -int64(i), 7, 1 << 62})
					r.Hit(Vals{int64(i)})
				}
				c.AddWork(uint64(n))
				return 0
			}
		})
	}

	quiet, busy := build(true, 1), build(false, 123456)

	if len(quiet) != len(busy) {
		t.Fatalf("line counts differ: %d vs %d", len(quiet), len(busy))
	}
	for i := range quiet {
		if len(quiet[i]) != len(busy[i]) {
			t.Errorf("line %d: %d vs %d bytes\n%s\n%s", i, len(quiet[i]), len(busy[i]), quiet[i], busy[i])
		}
	}
}

func TestTheEndLineCarriesTheExitStatusOnly(t *testing.T) {
	for _, status := range []int{0, 1, 3, 64, 255} {
		lines := reportOf(t, "p1", func(c *Catalog) func() int { return func() int { return status } })

		if err := checkTrial(lines); err != nil {
			t.Fatalf("status %d: %v", status, err)
		}
		if end := lines[len(lines)-1]; !strings.Contains(end, fmt.Sprintf(`"status":%*d`, numWidth, status)) {
			t.Fatalf("status %d not in %s", status, end)
		}
	}
}
