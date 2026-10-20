package sqlcancel

import (
	"testing"

	"github.com/spiculedata/hairspring/examples/go-sql-cancel/internal/retaindrv"
)

func row(qid int64, idx int) string { return string(retaindrv.AppendRow(nil, qid, idx)) }

func TestClassifyOwnRowIsClassOwn(t *testing.T) {
	v := Classify(7, row(7, 0))
	if v != (Verdict{Class: ClassOwn, Head: 7, Tail: 7}) {
		t.Fatalf("got %+v", v)
	}
}

func TestClassifyAnotherQuerysRowIsForeign(t *testing.T) {
	v := Classify(7, row(9, 0))
	if v != (Verdict{Class: ClassForeign, Head: 9, Tail: 9}) {
		t.Fatalf("got %+v", v)
	}
}

func TestClassifyAHalfAndHalfRowIsTorn(t *testing.T) {
	torn := row(7, 0)[:12] + row(9, 0)[12:]
	v := Classify(7, torn)
	if v != (Verdict{Class: ClassTorn, Head: 7, Tail: 9}) {
		t.Fatalf("got %+v", v)
	}
}

func TestClassifyUnparsableReportsMinusOneForBothIds(t *testing.T) {
	for name, in := range map[string]string{
		"empty":   "",
		"garbage": "xxxxxxxxxxxxxxxxxxxxxxxxx",
		"short":   row(7, 0)[:20],
	} {
		v := Classify(7, in)
		if v != (Verdict{Class: ClassUnparsable, Head: -1, Tail: -1}) {
			t.Errorf("%s: got %+v", name, v)
		}
	}
}

func TestClassifyOwnIdsButWrongRowIndexIsUnparsableNotOwn(t *testing.T) {
	v := Classify(7, row(7, 5))
	if v.Class != ClassUnparsable {
		t.Fatalf("got %+v", v)
	}
}
