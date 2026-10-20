package hs

import (
	"bytes"
	"encoding/json"
	"strings"
	"testing"
)

// decode strips the prefix and newline from a line and parses the JSON object, failing the test on any deviation.
func decode(t *testing.T, line []byte) map[string]any {
	t.Helper()
	if len(line) > maxLineBytes {
		t.Fatalf("line is %d bytes, cap is %d", len(line), maxLineBytes)
	}
	if !bytes.HasPrefix(line, []byte("HAIRSPRING_EVENT {")) || line[len(line)-1] != '\n' {
		t.Fatalf("bad framing: %q", line)
	}
	var m map[string]any
	if err := json.Unmarshal(line[len("HAIRSPRING_EVENT "):], &m); err != nil {
		t.Fatalf("line is not JSON: %v: %q", err, line)
	}
	if m["v"] != float64(1) {
		t.Fatalf("v = %v, want 1", m["v"])
	}
	return m
}

func TestHelloLine(t *testing.T) {
	m := decode(t, helloLine("p1", "00ff00ff00ff00ff"))

	if m["kind"] != "hello" || m["proc"] != "p1" || m["sdk"] != "go" || m["signal_handlers"] != false {
		t.Fatalf("hello = %v", m)
	}
	if m["catalog_hash"] != "00ff00ff00ff00ff" || m["sdk_version"] != SDKVersion {
		t.Fatalf("hello = %v", m)
	}
}

func TestCatalogLineCarriesTextAndLocation(t *testing.T) {
	s := Site{ID: "c.id", Kind: KindSometimes, Msg: `say "hi" <b>`, File: "dir/f.go", Line: 7, Func: "fn"}

	m := decode(t, catalogLine("p1", s))

	loc := m["loc"].(map[string]any)
	if m["type"] != "sometimes" || m["must_hit"] != true || m["msg"] != `say "hi" <b>` {
		t.Fatalf("catalog = %v", m)
	}
	if loc["file"] != "dir/f.go" || loc["line"] != float64(7) || loc["func"] != "fn" {
		t.Fatalf("loc = %v", loc)
	}
}

func TestCatalogLineOmitsEmptyTextAndMustHitFollowsKind(t *testing.T) {
	m := decode(t, catalogLine("p1", Site{ID: "c.u", Kind: KindUnreachable}))

	if _, has := m["msg"]; has {
		t.Errorf("empty msg should be omitted: %v", m)
	}
	if _, has := m["loc"]; has {
		t.Errorf("empty location should be omitted: %v", m)
	}
	if m["must_hit"] != false {
		t.Errorf("unreachable must_hit = %v, want false", m["must_hit"])
	}
}

func TestCatalogLineDropsTextWhenEscapingOverflowsTheLineCap(t *testing.T) {
	// '<' escapes to six bytes: 512 of them in the message and 256 in the file name are about 4600 bytes of
	// escaped text, over the cap, so the text goes and the id stays.
	s := Site{ID: "c.big", Kind: KindAlways, Msg: strings.Repeat("<", 512), File: strings.Repeat("<", 256), Func: strings.Repeat("g", 128)}

	m := decode(t, catalogLine("p1", s))

	if m["id"] != "c.big" {
		t.Fatalf("id lost: %v", m)
	}
	if _, has := m["msg"]; has {
		t.Fatalf("oversized text should have been dropped: %v", m)
	}
}

func TestAssertLinePadsCountersToConstantWidth(t *testing.T) {
	quiet := SiteReport{Site: Site{ID: "w.id", Kind: KindAlways}, Trues: 1}
	busy := SiteReport{
		Site: Site{ID: "w.id", Kind: KindAlways}, Trues: 411, Falses: 1,
		FirstFalse: &Witness{Thread: "go", Ord: 675, Vals: Vals{71, 8, 0, -9223372036854775808}},
		FirstTrue:  &Witness{Thread: "go", Ord: 3, Vals: Vals{0, 5, 0, 0}},
	}

	a := assertLine("p1", quiet)
	b := assertLine("p1", busy)

	if len(a) != len(b) {
		t.Fatalf("assert line length depends on the values: %d vs %d", len(a), len(b))
	}
	m := decode(t, b)
	ff := m["first_false"].(map[string]any)
	if m["hits"] != float64(412) || m["trues"] != float64(411) || m["falses"] != float64(1) {
		t.Fatalf("counts = %v", m)
	}
	if ff["ord"] != float64(675) || ff["thread"] != "go" || len(ff["vals"].([]any)) != 4 {
		t.Fatalf("first_false = %v", ff)
	}
	if m1 := decode(t, a); m1["first_false"] != nil {
		t.Fatalf("first_false = %v, want null", m1["first_false"])
	}
}

func TestAssertLineForPointKindsHasNoFirstFalse(t *testing.T) {
	r := SiteReport{Site: Site{ID: "p.id", Kind: KindReachable}, Trues: 2, FirstTrue: &Witness{Thread: "go", Ord: 1}}

	m := decode(t, assertLine("p1", r))

	if _, has := m["first_false"]; has {
		t.Fatalf("point kinds must not send first_false: %v", m)
	}
	if m["hits"] != float64(2) || m["falses"] != float64(0) {
		t.Fatalf("counts = %v", m)
	}
}

func TestWorkAndEndLines(t *testing.T) {
	w := decode(t, workLine("p1", 300))
	e := decode(t, endLine("p1", 3, 7, 414, 0))

	if w["kind"] != "work" || w["ops"] != float64(300) {
		t.Fatalf("work = %v", w)
	}
	if e["reason"] != "exit" || e["status"] != float64(3) || e["sites"] != float64(7) ||
		e["evals"] != float64(414) || e["dropped"] != float64(0) || e["threads"] != float64(0) {
		t.Fatalf("end = %v", e)
	}
}

func TestCatalogHashIsStableAndSensitiveToTheCatalog(t *testing.T) {
	build := func(kind Kind) Report {
		c := NewCatalog()
		mustDeclare(t, c, Site{ID: "h.one", Kind: kind})
		return c.Snapshot()
	}

	same1, same2, other := catalogHash(build(KindAlways)), catalogHash(build(KindAlways)), catalogHash(build(KindSometimes))

	if same1 != same2 || same1 == other || len(same1) != 16 {
		t.Fatalf("hashes: %q %q %q", same1, same2, other)
	}
}

func TestViolationLine(t *testing.T) {
	withMsg := violationLine(SiteReport{Site: Site{ID: "v.id", Kind: KindAlways, Msg: "row mixed up"}, Falses: 2})
	noMsg := violationLine(SiteReport{Site: Site{ID: "v.un", Kind: KindUnreachable}, Trues: 1})

	if string(withMsg) != "HAIRSPRING_VIOLATION: v.id: row mixed up (2 false)\n" {
		t.Errorf("with msg = %q", withMsg)
	}
	if string(noMsg) != "HAIRSPRING_VIOLATION: v.un: unreachable site hit (1 hit)\n" {
		t.Errorf("no msg = %q", noMsg)
	}
	bare := violationLine(SiteReport{Site: Site{ID: "v.bare", Kind: KindAlways}, Falses: 1})
	if string(bare) != "HAIRSPRING_VIOLATION: v.bare: always-type property was false (1 false)\n" {
		t.Errorf("bare = %q", bare)
	}
}
